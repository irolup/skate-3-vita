//! The 3D scene: the Course (one draw, baked vertex colours, fog) and the skinned
//! skater and board (one draw per material, GPU skinning).
use core::ffi::{CStr, c_void};

use glam::{Mat4, Vec3, Vec4};
use vita_pack::map::{self as park_pack, Render as ParkRender};
use vita_pack::skater::{self, AlphaMode, Skater};
use vita_pack::texture::{Format, Texture};

use crate::course::{WORLD_STRIDE, WorldVertex};
use crate::gl::Program;
use crate::math::{AMBIENT, DIFFUSE, SKY, light_direction};
use crate::vgl::*;

/// The skin palette size compiled into skin.vert.cg.
pub const MAX_JOINTS: usize = 40;
/// Linear fog range (m).
const FOG: [f32; 2] = [60.0, 160.0];

const fn cstr(s: &str) -> &CStr {
    match CStr::from_bytes_with_nul(s.as_bytes()) {
        Ok(c) => c,
        Err(_) => panic!("shader source must not contain NUL"),
    }
}
const WORLD_VERT: &CStr = cstr(concat!(include_str!("../../../shaders/world.vert.cg"), "\0"));
const WORLD_FRAG: &CStr = cstr(concat!(include_str!("../../../shaders/world.frag.cg"), "\0"));
const SKIN_VERT: &CStr = cstr(concat!(include_str!("../../../shaders/skin.vert.cg"), "\0"));
const SKIN_FRAG: &CStr = cstr(concat!(include_str!("../../../shaders/skin.frag.cg"), "\0"));
const SKIN_MASK_FRAG: &CStr = cstr(concat!(include_str!("../../../shaders/skin_mask.frag.cg"), "\0"));
const SKIN_FADE_FRAG: &CStr = cstr(concat!(include_str!("../../../shaders/skin_fade.frag.cg"), "\0"));
const PED_FRAG: &CStr = cstr(concat!(include_str!("../../../shaders/ped.frag.cg"), "\0"));
const PARK_VERT: &CStr = cstr(concat!(include_str!("../../../shaders/park.vert.cg"), "\0"));
const PARK_FRAG: &CStr = cstr(concat!(include_str!("../../../shaders/park.frag.cg"), "\0"));
const PARK_MASK_FRAG: &CStr = cstr(concat!(include_str!("../../../shaders/park_mask.frag.cg"), "\0"));
const PROP_VERT: &CStr = cstr(concat!(include_str!("../../../shaders/prop.vert.cg"), "\0"));
const CAR_FRAG: &CStr = cstr(concat!(include_str!("../../../shaders/car.frag.cg"), "\0"));
const ICON_VERT: &CStr = cstr(concat!(include_str!("../../../shaders/icon.vert.cg"), "\0"));
const ICON_FRAG: &CStr = cstr(concat!(include_str!("../../../shaders/icon.frag.cg"), "\0"));
const CAR_GLASS_FRAG: &CStr = cstr(concat!(include_str!("../../../shaders/car_glass.frag.cg"), "\0"));
/// Retail `kd` without a normal map (retail_world.wgsl).
const KD: f32 = 0.93429;
/// Park fog range (m); the far plane is `math::FAR`.
const PARK_FOG: [f32; 2] = [110.0, 195.0];

pub(crate) struct ParkProgram {
    pub(crate) program: Program,
    pub(crate) viewproj: GLint,
    pub(crate) eye: GLint,
    pub(crate) fog: GLint,
    pub(crate) tint: GLint,
    pub(crate) scale: GLint,
    pub(crate) horizon: GLint,
    pub(crate) cutoff: Option<GLint>,
    /// The props' model matrix; `None` for the world, whose vertices are in world space.
    pub(crate) model: Option<GLint>,
}

impl ParkProgram {
    fn new(name: &str, fragment: &CStr, masked: bool) -> Self {
        Self::with_vertex(name, PARK_VERT, fragment, masked, false)
    }

    fn with_vertex(name: &str, vertex: &CStr, fragment: &CStr, masked: bool, placed: bool) -> Self {
        let program = Program::new(name, vertex, fragment, &[c"aPosition", c"aUv", c"aLightmapUv"]);
        let p = Self {
            model: placed.then(|| program.uniform(c"model")),
            viewproj: program.uniform(c"viewproj"),
            eye: program.uniform(c"eye"),
            fog: program.uniform(c"fog"),
            tint: program.uniform(c"tint"),
            scale: program.uniform(c"scale"),
            horizon: program.uniform(c"horizon"),
            cutoff: masked.then(|| program.uniform(c"cutoff")),
            program,
        };
        unsafe {
            glUseProgram(p.program.id);
            glUniform1i(p.program.uniform(c"albedo"), 0);
            glUniform1i(p.program.uniform(c"lightmap"), 1);
        }
        p
    }
}

struct ParkDraw {
    vertex_offset: usize,
    first_index: u32,
    index_count: u32,
    /// Indices into `ParkGpu::textures` (None: the white texture).
    albedo: Option<usize>,
    lightmap: Option<usize>,
    tint: [f32; 4],
    cutoff: Option<f32>,
    min: Vec3,
    max: Vec3,
}

struct ParkGpu {
    vbo: GLuint,
    ibo: GLuint,
    /// GL names, filled in as the textures upload (see `Scene::upload_pending`).
    textures: Vec<GLuint>,
    /// Textures still to upload, in order; each is dropped once uploaded.
    pending: Vec<Option<Texture>>,
    uploaded: usize,
    draws: Vec<ParkDraw>,
    /// Horizon in display space (clear and fog colour).
    horizon: [f32; 3],
    scale: f32,
}

struct WorldProgram {
    program: Program,
    viewproj: GLint,
    eye: GLint,
    fog: GLint,
    fog_color: GLint,
}

struct SkinProgram {
    program: Program,
    viewproj: GLint,
    bones: GLint,
    light_dir: GLint,
    light: GLint,
    tint: GLint,
    cutoff: Option<GLint>,
    /// The cars' paint pair (`red` = secondary, `blue` = chassis).
    paint: Option<(GLint, GLint)>,
}

impl SkinProgram {
    fn new(name: &str, fragment: &CStr, masked: bool) -> Self {
        Self::with_paint(name, fragment, masked, false)
    }

    fn with_paint(name: &str, fragment: &CStr, masked: bool, painted: bool) -> Self {
        let program = Program::new(
            name,
            SKIN_VERT,
            fragment,
            &[c"aPosition", c"aNormal", c"aTexcoord", c"aJoints", c"aWeights"],
        );
        let s = Self {
            viewproj: program.uniform(c"viewproj"),
            bones: program.uniform(c"bones"),
            light_dir: program.uniform(c"lightDir"),
            light: program.uniform(c"light"),
            tint: program.uniform(c"tint"),
            cutoff: masked.then(|| program.uniform(c"cutoff")),
            paint: painted.then(|| (program.uniform(c"red"), program.uniform(c"blue"))),
            program,
        };
        unsafe {
            glUseProgram(s.program.id);
            glUniform1i(s.program.uniform(c"tex"), 0);
            let l = light_direction();
            glUniform3f(s.light_dir, l.x, l.y, l.z);
            glUniform2f(s.light, AMBIENT, DIFFUSE);
        }
        s
    }
}

struct Draw {
    first_index: u32,
    index_count: u32,
    texture: GLuint,
    tint: [f32; 4],
    cutoff: Option<f32>,
    double_sided: bool,
}

struct SkaterGpu {
    vbo: GLuint,
    ibo: GLuint,
    draws: Vec<Draw>,
}

pub struct Scene {
    world: WorldProgram,
    skin: SkinProgram,
    skin_mask: SkinProgram,
    park: ParkProgram,
    park_mask: ParkProgram,
    course_vbo: GLuint,
    course_vertices: GLsizei,
    white: GLuint,
    skater: Option<SkaterGpu>,
    /// The loaded retail park; the Course is drawn when there is none.
    park_gpu: Option<ParkGpu>,
    /// R2: the Trick Guide's demo set (drawn instead of the world by `draw_demo` while the guide is open).
    demo_gpu: Option<ParkGpu>,
    /// A campaign district: tiles streamed in while skating; drawn instead of a park.
    district: Option<crate::district::DistrictGpu>,
    /// Draws submitted / culled in the last frame.
    pub last_draws: (u32, u32),
    /// Per pass of `draw` since `take_pass_times`: µs, draws, and the frames.
    pass_us: [u64; 5],
    pass_draws: [u64; 5],
    pass_frames: u32,
    /// The world's movable props (template meshes) and their programs.
    props: Option<PropsGpu>,
    prop: ParkProgram,
    prop_mask: ParkProgram,
    /// The traffic car models (by `Session::car_models` index) and their programs.
    cars: Vec<Option<CarGpu>>,
    car: SkinProgram,
    car_glass: SkinProgram,
    /// NPC skaters fading in or out (the skin program with an opacity).
    skin_fade: SkinProgram,
    skin_fade_alpha: GLint,
    /// The ped models (by `Session::ped_models` index) and their program (paint, fade).
    peds: Vec<Option<PedGpu>>,
    /// The ped hand prop models (by `Session::hand_prop_models` index; rigid, one joint), drawn
    /// with the ped program.
    hand_props: Vec<Option<PedGpu>>,
    ped: SkinProgram,
    ped_paint: GLint,
    ped_fade: GLint,
    /// The colour grade the programs have (uploaded again when it changes).
    grade: Grade,
    grade_dirty: bool,
    /// The teleporter icons' mesh and textures (`load_icons`).
    icons: Option<IconsGpu>,
}

/// The teleporter icon's ribbon mesh (position, uv; `IconsGpu::STRIDE`) and its two pass textures, with its program.
struct IconsGpu {
    program: Program,
    viewproj: GLint,
    model: GLint,
    scale: GLint,
    vbo: GLuint,
    ibo: GLuint,
    index_count: GLsizei,
    front: GLuint,
    background: GLuint,
    /// The called skaters' trails (`set_trail`): world-space ribbons (vbo, ibo, index count) by slot.
    trails: [Option<(GLuint, GLuint, GLsizei)>; TRAIL_SLOTS],
}

impl IconsGpu {
    const STRIDE: usize = 20;
}

/// Trails at most (retail keeps 4 trail objects, `827ADA30`).
pub const TRAIL_SLOTS: usize = 4;

/// One called skater's trail to draw: its slot (`Scene::set_trail`) and the pass factors as [`IconInstance`]'s.
#[derive(Clone, Copy, Debug)]
pub struct TrailInstance {
    pub slot: usize,
    pub front: f32,
    pub background: [f32; 2],
}

/// One teleporter icon to draw: its model matrix and the pass factors (retail's `i_intensity` x the fade).
#[derive(Clone, Copy, Debug)]
pub struct IconInstance {
    pub model: Mat4,
    /// Front pass rgb factor (I.x x fade) and background pass rgb factor (I.y) and alpha factor (sqrt(I.y x fade)).
    pub front: f32,
    pub background: [f32; 2],
}

/// The colour grade (retail post FX colour matrix, `skate_core::colour_matrix::build`; research b100 / b101): near and
/// far 3x4 rows, weighted linearly in view depth between the record's distances (full near up to `distance_near`,
/// full far from `distance_far`). Retail applies it in its tone pass over the frame; here each world shader applies it
/// last (the same for opaque draws; alpha blends of affine maps commute), so the HUD and the menus stay ungraded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grade {
    pub near: [[f32; 4]; 3],
    pub far: [[f32; 4]; 3],
    pub distance_near: f32,
    pub distance_far: f32,
}

impl Grade {
    pub const IDENTITY: Self = Self {
        near: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
        far: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
        distance_near: 50.0,
        distance_far: 100.0,
    };

    /// The far band on a colour (the sky, behind everything).
    fn far(&self, rgb: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|i| {
            let r = self.far[i];
            (r[0] * rgb[0] + r[1] * rgb[1] + r[2] * rgb[2] + r[3]).clamp(0.0, 1.0)
        })
    }
}

/// One ped to draw this frame.
#[derive(Clone, Copy, Debug)]
pub struct PedInstance<'a> {
    pub model: u16,
    /// Per skin joint: root * joint global * inverse bind.
    pub palette: &'a [Mat4],
    pub center: Vec3,
    pub tint_a: [f32; 4],
    pub tint_b: [f32; 4],
    pub alpha: f32,
    pub lod: u8,
}

struct PedGpu {
    vbo: GLuint,
    ibo: GLuint,
    textures: Vec<GLuint>,
    /// Each draw with its LOD (materials named `LOD1:` by the bake) and tint flag (`TINT:`).
    draws: Vec<(Draw, u8, bool)>,
}

/// One NPC skater to draw this frame: the skater mesh with its own palette and opacity.
#[derive(Clone, Copy, Debug)]
pub struct SkinInstance<'a> {
    /// Per skin joint: root * joint global * inverse bind (at most `MAX_JOINTS`).
    pub palette: &'a [Mat4],
    pub alpha: f32,
    /// Its root in the world: the skater is culled as a 2 m sphere around it.
    pub center: Vec3,
}

/// One traffic car to draw this frame.
#[derive(Clone, Copy, Debug)]
pub struct CarInstance {
    /// Index of its model in the loaded car list.
    pub model: u16,
    pub transform: Mat4,
    /// The wheels' spin about their x axis (rad).
    pub wheel: f32,
    pub chassis: [f32; 4],
    pub secondary: [f32; 4],
}

/// The six wheel joints of the car rig (upstream `living_world::vehicles::WHEEL_BONES`).
const WHEEL_JOINTS: [&str; 6] = ["LeftFront_wheel", "RightFront_wheel", "LeftRear_wheel_1", "RightRear_wheel_1", "LeftRear_wheel_2", "RightRear_wheel_2"];

struct CarGpu {
    vbo: GLuint,
    ibo: GLuint,
    textures: Vec<GLuint>,
    /// Body draws, then the glass (blended).
    draws: Vec<(Draw, bool)>,
    /// Per joint: (wheel, bind = inverse of the inverse bind, inverse bind).
    joints: Vec<(bool, Mat4, Mat4)>,
    min: Vec3,
    max: Vec3,
}

/// One movable prop to draw this frame: its template (index in the props render) and pose.
#[derive(Clone, Copy, Debug)]
pub struct PropInstance {
    pub template: u16,
    pub model: Mat4,
}

struct PropsGpu {
    vbo: GLuint,
    ibo: GLuint,
    textures: Vec<GLuint>,
    pending: Vec<Option<Texture>>,
    uploaded: usize,
    draws: Vec<ParkDraw>,
    /// Per template: its draws and the bounds of their vertices (template space).
    templates: Vec<(std::ops::Range<usize>, Vec3, Vec3)>,
    horizon: [f32; 3],
    scale: f32,
}

/// One frame's inputs.
pub struct Frame<'a> {
    pub view_proj: Mat4,
    pub eye: Vec3,
    /// Per skin joint: root * joint global * inverse bind (at most `MAX_JOINTS`).
    pub palette: &'a [Mat4],
    /// The movable props streamed in, at their poses.
    pub props: &'a [PropInstance],
    /// The traffic cars.
    pub cars: &'a [CarInstance],
    /// The NPC skaters (the skater mesh again, each with its palette).
    pub npcs: &'a [SkinInstance<'a>],
    /// The peds in view.
    pub peds: &'a [PedInstance<'a>],
    /// The hand props (held or released), each a one-matrix palette.
    pub hand_props: &'a [PedInstance<'a>],
    /// The teleporter icons in view (drawn last, blended).
    pub icons: &'a [IconInstance],
    /// The called skaters' trails in view (drawn with the icons).
    pub trails: &'a [TrailInstance],
}

impl Scene {
    /// Compiles (or loads cached) programs and uploads the Course mesh.
    pub fn new(course: &[WorldVertex]) -> Self {
        let program = Program::new("world", WORLD_VERT, WORLD_FRAG, &[c"aPosition", c"aColor"]);
        let world = WorldProgram {
            viewproj: program.uniform(c"viewproj"),
            eye: program.uniform(c"eye"),
            fog: program.uniform(c"fog"),
            fog_color: program.uniform(c"fogColor"),
            program,
        };
        let skin = SkinProgram::new("skin", SKIN_FRAG, false);
        let skin_mask = SkinProgram::new("skin_mask", SKIN_MASK_FRAG, true);
        let park = ParkProgram::new("park", PARK_FRAG, false);
        let park_mask = ParkProgram::new("park_mask", PARK_MASK_FRAG, true);
        let prop = ParkProgram::with_vertex("prop", PROP_VERT, PARK_FRAG, false, true);
        let prop_mask = ParkProgram::with_vertex("prop_mask", PROP_VERT, PARK_MASK_FRAG, true, true);
        let car = SkinProgram::with_paint("car", CAR_FRAG, false, true);
        let car_glass = SkinProgram::new("car_glass", CAR_GLASS_FRAG, false);
        let skin_fade = SkinProgram::new("skin_fade", SKIN_FADE_FRAG, true);
        let skin_fade_alpha = skin_fade.program.uniform(c"fade");
        let ped = SkinProgram::with_paint("ped", PED_FRAG, true, true);
        let (ped_paint, ped_fade) = (ped.program.uniform(c"paint"), ped.program.uniform(c"fade"));
        let mut bytes = Vec::with_capacity(course.len() * WORLD_STRIDE);
        for v in course {
            for c in v.position {
                bytes.extend_from_slice(&c.to_le_bytes());
            }
            bytes.extend_from_slice(&v.color);
        }
        unsafe {
            let course_vbo = buffer(GL_ARRAY_BUFFER, &bytes);
            let white = upload_texture(&Texture { width: 1, height: 1, format: Format::Rgba8, mips: vec![vec![255; 4]] });
            Self {
                world,
                skin,
                skin_mask,
                park,
                park_mask,
                course_vbo,
                course_vertices: course.len() as GLsizei,
                white,
                skater: None,
                park_gpu: None,
                demo_gpu: None,
                district: None,
                last_draws: (0, 0),
                pass_us: [0; 5],
                pass_draws: [0; 5],
                pass_frames: 0,
                props: None,
                prop,
                prop_mask,
                cars: Vec::new(),
                car,
                car_glass,
                skin_fade,
                skin_fade_alpha,
                peds: Vec::new(),
                hand_props: Vec::new(),
                ped,
                ped_paint,
                ped_fade,
                grade: Grade::IDENTITY,
                grade_dirty: true,
                icons: None,
            }
        }
    }

    /// Uploads the skater pack's mesh and textures.
    pub fn load_skater(&mut self, s: &Skater) {
        unsafe {
            let vbo = buffer(GL_ARRAY_BUFFER, &s.vertex_data);
            let index_bytes: Vec<u8> = s.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
            let ibo = buffer(GL_ELEMENT_ARRAY_BUFFER, &index_bytes);
            let textures: Vec<GLuint> = s.textures.iter().map(|t| upload_texture(t)).collect();
            // Opaque materials first, masked ones last (alpha test sorted
            // last). Blend materials are drawn as masked at 0.5.
            let mut draws: Vec<Draw> = s
                .primitives
                .iter()
                .map(|p| {
                    let m = &s.materials[usize::from(p.material)];
                    Draw {
                        first_index: p.first_index,
                        index_count: p.index_count,
                        texture: m.texture.map_or(self.white, |t| textures[usize::from(t)]),
                        tint: m.base_color,
                        cutoff: match m.alpha_mode {
                            AlphaMode::Opaque => None,
                            AlphaMode::Mask => Some(m.alpha_cutoff),
                            AlphaMode::Blend => Some(0.5),
                        },
                        double_sided: m.double_sided,
                    }
                })
                .collect();
            draws.sort_by_key(|d| d.cutoff.is_some());
            self.skater = Some(SkaterGpu { vbo, ibo, draws });
        }
    }

    /// Loads the traffic car models (`None`: a model that did not load; its cars are not
    /// drawn), replacing any loaded ones.
    pub fn load_cars(&mut self, models: &[Option<Skater>]) {
        self.unload_cars();
        self.cars = models
            .iter()
            .map(|m| {
                let s = m.as_ref()?;
                unsafe {
                    let vbo = buffer(GL_ARRAY_BUFFER, &s.vertex_data);
                    let index_bytes: Vec<u8> = s.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
                    let ibo = buffer(GL_ELEMENT_ARRAY_BUFFER, &index_bytes);
                    glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
                    let textures: Vec<GLuint> = s.textures.iter().map(upload_texture).collect();
                    let mut draws: Vec<(Draw, bool)> = s
                        .primitives
                        .iter()
                        .map(|p| {
                            let m = &s.materials[usize::from(p.material)];
                            let draw = Draw {
                                first_index: p.first_index,
                                index_count: p.index_count,
                                texture: m.texture.map_or(self.white, |t| textures[usize::from(t)]),
                                tint: m.base_color,
                                cutoff: None,
                                double_sided: m.double_sided,
                            };
                            (draw, m.alpha_mode == AlphaMode::Blend)
                        })
                        .collect();
                    draws.sort_by_key(|d| d.1);
                    let joints = s
                        .joints
                        .iter()
                        .map(|j| {
                            let inverse_bind = Mat4::from_cols_array(&j.inverse_bind);
                            (WHEEL_JOINTS.contains(&j.name.as_str()), inverse_bind.inverse(), inverse_bind)
                        })
                        .collect();
                    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                    for v in s.vertex_data.chunks_exact(skater::STRIDE) {
                        let at = |o: usize| f32::from_le_bytes(v[skater::POSITION + o..skater::POSITION + o + 4].try_into().unwrap());
                        let p = Vec3::new(at(0), at(4), at(8));
                        min = min.min(p);
                        max = max.max(p);
                    }
                    Some(CarGpu { vbo, ibo, textures, draws, joints, min, max })
                }
            })
            .collect();
    }

    /// Loads the ped models (`None`: a model that did not load), replacing any loaded ones.
    pub fn load_peds(&mut self, models: &[Option<Skater>]) {
        self.unload_peds();
        let white = self.white;
        self.peds = models.iter().map(|m| Some(ped_gpu(white, m.as_ref()?))).collect();
    }

    /// Loads the hand prop models, replacing any loaded ones.
    pub fn load_hand_props(&mut self, models: &[Option<Skater>]) {
        for c in self.hand_props.drain(..).flatten() {
            unsafe { free_ped_gpu(&c) };
        }
        let white = self.white;
        self.hand_props = models.iter().map(|m| Some(ped_gpu(white, m.as_ref()?))).collect();
    }

    /// Frees the ped models.
    pub fn unload_peds(&mut self) {
        for c in self.peds.drain(..).flatten() {
            unsafe {
                glDeleteBuffers(1, &c.vbo);
                glDeleteBuffers(1, &c.ibo);
                glDeleteTextures(c.textures.len() as GLsizei, c.textures.as_ptr());
            }
        }
    }

    /// The peds in view (culled as a 2 m sphere round their root): their LOD's draws, the body
    /// painted with the tint pair, blended while fading.
    fn draw_peds(&self, instances: &[PedInstance], models: &[Option<PedGpu>], view_proj: &Mat4, vp: &[f32; 16]) -> (u32, u32) {
        if instances.is_empty() || models.is_empty() {
            return (0, 0);
        }
        let planes = frustum(view_proj);
        let p = &self.ped;
        let (mut drawn, mut culled) = (0, 0);
        unsafe {
            glUseProgram(p.program.id);
            glUniformMatrix4fv(p.viewproj, 1, GL_FALSE, vp.as_ptr());
            glActiveTexture(GL_TEXTURE0);
            let mut palette = [0f32; 16 * MAX_JOINTS];
            for ped in instances {
                let Some(Some(gpu)) = models.get(usize::from(ped.model)) else { continue };
                let c = ped.center + Vec3::Y;
                if ped.alpha <= 0.0 || !planes.iter().all(|pl| pl.truncate().dot(c) + pl.w >= -2.0 * pl.truncate().length()) {
                    culled += 1;
                    continue;
                }
                let joints = ped.palette.len().min(MAX_JOINTS);
                for (slot, m) in palette.chunks_mut(16).zip(&ped.palette[..joints]) {
                    slot.copy_from_slice(&m.to_cols_array());
                }
                glUniformMatrix4fv(p.bones, joints as GLsizei, GL_FALSE, palette.as_ptr());
                if let Some((red, blue)) = p.paint {
                    glUniform4f(red, ped.tint_a[0], ped.tint_a[1], ped.tint_a[2], ped.tint_a[3]);
                    glUniform4f(blue, ped.tint_b[0], ped.tint_b[1], ped.tint_b[2], ped.tint_b[3]);
                }
                let fading = ped.alpha < 1.0;
                glUniform1f(self.ped_fade, ped.alpha.min(1.0));
                if fading {
                    glEnable(GL_BLEND);
                    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
                }
                glBindBuffer(GL_ARRAY_BUFFER, gpu.vbo);
                glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, gpu.ibo);
                attributes(&[
                    (3, GL_FLOAT, GL_FALSE, skater::POSITION),
                    (3, GL_BYTE, GL_TRUE, skater::NORMAL),
                    (2, GL_FLOAT, GL_FALSE, skater::UV),
                    (4, GL_UNSIGNED_BYTE, GL_FALSE, skater::JOINTS),
                    (4, GL_UNSIGNED_BYTE, GL_TRUE, skater::WEIGHTS),
                ], skater::STRIDE);
                for (d, lod, tinted) in &gpu.draws {
                    if *lod != ped.lod {
                        continue;
                    }
                    glUniform1f(self.ped_paint, if *tinted { 1.0 } else { 0.0 });
                    if let Some(location) = p.cutoff {
                        glUniform1f(location, d.cutoff.unwrap_or(-1.0));
                    }
                    glUniform4f(p.tint, d.tint[0], d.tint[1], d.tint[2], d.tint[3]);
                    if d.double_sided {
                        glDisable(GL_CULL_FACE);
                    } else {
                        glEnable(GL_CULL_FACE);
                    }
                    glBindTexture(GL_TEXTURE_2D, d.texture);
                    glDrawElements(GL_TRIANGLES, d.index_count as GLsizei, GL_UNSIGNED_SHORT, (d.first_index as usize * 2) as *const c_void);
                }
                if fading {
                    glDisable(GL_BLEND);
                }
                drawn += 1;
            }
            glDisable(GL_CULL_FACE);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        }
        (drawn, culled)
    }

    /// Frees the car models.
    pub fn unload_cars(&mut self) {
        for c in self.cars.drain(..).flatten() {
            unsafe {
                glDeleteBuffers(1, &c.vbo);
                glDeleteBuffers(1, &c.ibo);
                glDeleteTextures(c.textures.len() as GLsizei, c.textures.as_ptr());
            }
        }
    }

    /// Uploads a retail park (replacing any loaded one); the Course is no longer drawn.
    /// Creates the park's buffers and queues its textures; the textures upload a few
    /// per frame through `upload_pending` (a whole park in one frame took 0.1-0.4 s),
    /// and the park is drawn once all of them are in.
    pub fn load_park(&mut self, park: ParkRender) {
        self.unload_park();
        self.park_gpu = Some(park_gpu(park));
    }

    /// R2: the Trick Guide's demo set (a small park render: its textures upload now, the screen is black then).
    pub fn load_demo(&mut self, set: ParkRender) {
        self.unload_demo();
        let mut gpu = park_gpu(set);
        for (slot, texture) in gpu.textures.iter_mut().zip(gpu.pending.iter_mut()) {
            if let Some(texture) = texture.take() {
                *slot = upload_texture(&texture);
            }
        }
        gpu.uploaded = gpu.pending.len();
        self.demo_gpu = Some(gpu);
    }

    pub fn unload_demo(&mut self) {
        if let Some(p) = self.demo_gpu.take() {
            delete_park(p);
        }
    }

    pub fn demo_loaded(&self) -> bool {
        self.demo_gpu.is_some()
    }

    /// Clears and draws the Trick Guide demo: its set and the skater (`frame.palette`), nothing of the world.
    pub fn draw_demo(&mut self, frame: &Frame) {
        self.upload_grade();
        let Some(set) = &self.demo_gpu else { return };
        unsafe {
            glClearColor(set.horizon[0], set.horizon[1], set.horizon[2], 1.0);
            glClearDepthf(1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LEQUAL);
            glDepthMask(GL_TRUE);
            glDisable(GL_BLEND);
            glFrontFace(GL_CCW);
            glCullFace(GL_BACK);
            let vp = frame.view_proj.to_cols_array();
            self.last_draws = draw_park(set, self.white, &self.park, &self.park_mask, frame, &vp);
            if self.skater.is_some() {
                self.draw_skin(frame.palette, 1.0, &vp);
            }
        }
    }

    /// Loads the world's movable props (their template meshes and textures).
    pub fn load_props(&mut self, props: vita_pack::map::PropRender) {
        self.unload_props();
        let render = props.render;
        unsafe {
            let vbo = buffer(GL_ARRAY_BUFFER, &render.vertex_data);
            let index_bytes: Vec<u8> = render.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
            let ibo = buffer(GL_ELEMENT_ARRAY_BUFFER, &index_bytes);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
            let draws: Vec<ParkDraw> = render.draws.iter().map(park_draw).collect();
            let templates = props
                .templates
                .iter()
                .map(|t| {
                    let range = t.draws[0] as usize..t.draws[1] as usize;
                    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                    for d in &draws[range.clone()] {
                        lo = lo.min(d.min);
                        hi = hi.max(d.max);
                    }
                    (range, lo, hi)
                })
                .collect();
            // The props' textures are few and small: uploaded now (they can arrive after the
            // park's upload finished, or with no park at all in a district).
            let textures: Vec<GLuint> = render.textures.iter().map(upload_texture).collect();
            let uploaded = textures.len();
            self.props = Some(PropsGpu {
                vbo,
                ibo,
                textures,
                pending: (0..uploaded).map(|_| None).collect(),
                uploaded,
                draws,
                templates,
                horizon: [0.0; 3],
                scale: KD * render.exposure,
            });
        }
    }

    /// Frees the movable props.
    pub fn unload_props(&mut self) {
        if let Some(p) = self.props.take() {
            unsafe {
                glDeleteBuffers(1, &p.vbo);
                glDeleteBuffers(1, &p.ibo);
                let uploaded: Vec<GLuint> = p.textures.iter().copied().filter(|&t| t != 0).collect();
                glDeleteTextures(uploaded.len() as GLsizei, uploaded.as_ptr());
            }
        }
    }

    /// Uploads queued park textures until `budget` is spent (at least one per call).
    /// Returns the number still waiting.
    pub fn upload_pending(&mut self, budget: std::time::Duration) -> usize {
        let Some(park) = &mut self.park_gpu else { return 0 };
        let start = std::time::Instant::now();
        while park.uploaded < park.pending.len() {
            let i = park.uploaded;
            if let Some(texture) = park.pending[i].take() {
                park.textures[i] = upload_texture(&texture);
            }
            park.uploaded += 1;
            if start.elapsed() >= budget {
                break;
            }
        }
        park.pending.len() - park.uploaded
    }

    /// Starts a district: no tile yet, sky until they arrive (the streamer adds them).
    pub fn load_district(&mut self, tile_size: f32, horizon: [f32; 3], exposure: f32) {
        self.unload_park();
        self.unload_district();
        self.district = Some(crate::district::DistrictGpu::new(tile_size, horizon.map(|c| c.max(0.0).powf(1.0 / 2.2)), KD * exposure));
    }

    pub fn district_mut(&mut self) -> Option<&mut crate::district::DistrictGpu> {
        self.district.as_mut()
    }

    pub fn unload_district(&mut self) {
        if let Some(mut d) = self.district.take() {
            d.free();
        }
    }

    /// Frees the loaded park (back to the Course).
    pub fn unload_park(&mut self) {
        if let Some(p) = self.park_gpu.take() {
            delete_park(p);
        }
    }

    /// The teleporter icons' ribbon mesh (x, y, z, u, v per vertex; triangle indices) and its front and background
    /// textures (RGBA8), replacing any loaded ones.
    pub fn load_icons(&mut self, vertices: &[[f32; 5]], indices: &[u16], front: &Texture, background: &Texture) {
        self.unload_icons();
        let program = Program::new("icon", ICON_VERT, ICON_FRAG, &[c"aPosition", c"aUv"]);
        let bytes: Vec<u8> = vertices.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
        let index_bytes: Vec<u8> = indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        unsafe {
            glUseProgram(program.id);
            glUniform1i(program.uniform(c"tex"), 0);
            self.icons = Some(IconsGpu {
                viewproj: program.uniform(c"viewproj"),
                model: program.uniform(c"model"),
                scale: program.uniform(c"scale"),
                program,
                vbo: buffer(GL_ARRAY_BUFFER, &bytes),
                ibo: buffer(GL_ELEMENT_ARRAY_BUFFER, &index_bytes),
                index_count: indices.len() as GLsizei,
                front: upload_texture(front),
                background: upload_texture(background),
                trails: Default::default(),
            });
        }
    }

    /// A called skater's trail mesh (world space, the icons' vertex layout) in `slot`, replacing the last one; an empty
    /// mesh frees the slot. Needs the icons loaded (their program and textures).
    pub fn set_trail(&mut self, slot: usize, vertices: &[[f32; 5]], indices: &[u16]) {
        let Some(gpu) = self.icons.as_mut().filter(|_| slot < TRAIL_SLOTS) else { return };
        if let Some((vbo, ibo, _)) = gpu.trails[slot].take() {
            unsafe {
                glDeleteBuffers(1, &vbo);
                glDeleteBuffers(1, &ibo);
            }
        }
        if indices.is_empty() {
            return;
        }
        let bytes: Vec<u8> = vertices.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
        let index_bytes: Vec<u8> = indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        gpu.trails[slot] = Some(unsafe { (buffer(GL_ARRAY_BUFFER, &bytes), buffer(GL_ELEMENT_ARRAY_BUFFER, &index_bytes), indices.len() as GLsizei) });
    }

    pub fn unload_icons(&mut self) {
        if let Some(i) = self.icons.take() {
            unsafe {
                for (vbo, ibo, _) in i.trails.iter().flatten() {
                    glDeleteBuffers(1, vbo);
                    glDeleteBuffers(1, ibo);
                }
                glDeleteBuffers(1, &i.vbo);
                glDeleteBuffers(1, &i.ibo);
                glDeleteTextures(1, &i.front);
                glDeleteTextures(1, &i.background);
            }
        }
    }

    /// Retail's two passes per icon (b535 `827AFC80`, b536 `82CE9080`): the background alpha blended, then the front
    /// added; depth tested, no depth write, both faces.
    fn draw_icons(&self, icons: &[IconInstance], trails: &[TrailInstance], vp: &[f32; 16]) {
        let Some(gpu) = self.icons.as_ref().filter(|_| !icons.is_empty() || !trails.is_empty()) else { return };
        unsafe {
            // Background alpha blended, then front added, with the bound mesh.
            let passes = |model: &[f32; 16], front: f32, [b, a]: [f32; 2], count: GLsizei| {
                glUniformMatrix4fv(gpu.model, 1, GL_FALSE, model.as_ptr());
                glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
                glBindTexture(GL_TEXTURE_2D, gpu.background);
                glUniform4f(gpu.scale, b, b, b, a);
                glDrawElements(GL_TRIANGLES, count, GL_UNSIGNED_SHORT, std::ptr::null());
                glBlendFunc(GL_ONE, GL_ONE);
                glBindTexture(GL_TEXTURE_2D, gpu.front);
                glUniform4f(gpu.scale, front, front, front, 1.0);
                glDrawElements(GL_TRIANGLES, count, GL_UNSIGNED_SHORT, std::ptr::null());
            };
            glUseProgram(gpu.program.id);
            glUniformMatrix4fv(gpu.viewproj, 1, GL_FALSE, vp.as_ptr());
            glDisable(GL_CULL_FACE);
            glDepthMask(GL_FALSE);
            glEnable(GL_BLEND);
            glActiveTexture(GL_TEXTURE0);
            if !icons.is_empty() {
                glBindBuffer(GL_ARRAY_BUFFER, gpu.vbo);
                glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, gpu.ibo);
                attributes(&[(3, GL_FLOAT, GL_FALSE, 0), (2, GL_FLOAT, GL_FALSE, 12)], IconsGpu::STRIDE);
                for icon in icons {
                    passes(&icon.model.to_cols_array(), icon.front, icon.background, gpu.index_count);
                }
            }
            // The trails are in world space.
            let identity = Mat4::IDENTITY.to_cols_array();
            for trail in trails {
                let Some(Some((vbo, ibo, count))) = gpu.trails.get(trail.slot) else { continue };
                glBindBuffer(GL_ARRAY_BUFFER, *vbo);
                glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, *ibo);
                attributes(&[(3, GL_FLOAT, GL_FALSE, 0), (2, GL_FLOAT, GL_FALSE, 12)], IconsGpu::STRIDE);
                passes(&identity, trail.front, trail.background, *count);
            }
            glDisable(GL_BLEND);
            glDepthMask(GL_TRUE);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        }
    }

    /// The colour grade from now on (`Grade::IDENTITY`: none).
    pub fn set_grade(&mut self, grade: Grade) {
        if grade != self.grade {
            self.grade = grade;
            self.grade_dirty = true;
        }
    }

    /// Uploads the grade to every world program when it changed (a program keeps its uniforms).
    fn upload_grade(&mut self) {
        if !std::mem::take(&mut self.grade_dirty) {
            return;
        }
        let g = &self.grade;
        let rows: Vec<f32> = g.near.iter().chain(&g.far).flatten().copied().collect();
        let depth = [g.distance_near, 1.0 / (g.distance_far - g.distance_near).max(1e-3)];
        let programs = [
            &self.world.program,
            &self.skin.program,
            &self.skin_mask.program,
            &self.park.program,
            &self.park_mask.program,
            &self.prop.program,
            &self.prop_mask.program,
            &self.car.program,
            &self.car_glass.program,
            &self.skin_fade.program,
            &self.ped.program,
        ];
        unsafe {
            for p in programs {
                glUseProgram(p.id);
                glUniform4fv(p.uniform(c"grade"), 6, rows.as_ptr());
                glUniform2f(p.uniform(c"gradeDepth"), depth[0], depth[1]);
            }
        }
    }

    /// Clears and draws the 3D scene (the UI and the swap come after).
    pub fn draw(&mut self, frame: &Frame) {
        self.upload_grade();
        let sky = self.district.as_ref().map(|d| d.horizon()).or(self.park_gpu.as_ref().map(|p| p.horizon)).unwrap_or(SKY);
        let sky = self.grade.far(sky);
        unsafe {
            glClearColor(sky[0], sky[1], sky[2], 1.0);
            glClearDepthf(1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LEQUAL);
            glDepthMask(GL_TRUE);
            glDisable(GL_BLEND);
            glFrontFace(GL_CCW);
            glCullFace(GL_BACK);
            let vp = frame.view_proj.to_cols_array();
            // Diagnostics: each pass's time and draws (`take_pass_times`).
            let mark = |slot: usize, at: &mut std::time::Instant, us: &mut [u64; 5], draws: &mut [u32; 5], drawn: u32| {
                us[slot] += at.elapsed().as_micros() as u64;
                draws[slot] += drawn;
                *at = std::time::Instant::now();
            };
            let (mut at, mut us, mut draws) = (std::time::Instant::now(), [0u64; 5], [0u32; 5]);

            if let Some(district) = &mut self.district {
                self.last_draws = district.draw(&self.park, &self.park_mask, self.white, &frame.view_proj, frame.eye);
            } else if let Some(park) = &self.park_gpu {
                // Until every texture is in, only the sky shows (no white flash).
                self.last_draws = if park.uploaded == park.pending.len() {
                    draw_park(park, self.white, &self.park, &self.park_mask, frame, &vp)
                } else {
                    (0, 0)
                };
            } else {
            // Course: double sided like skate-game's test world.
            glDisable(GL_CULL_FACE);
            glUseProgram(self.world.program.id);
            glUniformMatrix4fv(self.world.viewproj, 1, GL_FALSE, vp.as_ptr());
            glUniform3f(self.world.eye, frame.eye.x, frame.eye.y, frame.eye.z);
            glUniform2f(self.world.fog, FOG[0], FOG[1]);
            glUniform3f(self.world.fog_color, SKY[0], SKY[1], SKY[2]);
            glBindBuffer(GL_ARRAY_BUFFER, self.course_vbo);
            attributes(&[
                (3, GL_FLOAT, GL_FALSE, 0),
                (4, GL_UNSIGNED_BYTE, GL_TRUE, 12),
            ], WORLD_STRIDE);
            glDrawArrays(GL_TRIANGLES, 0, self.course_vertices);
            self.last_draws = (1, 0);
            }
            mark(0, &mut at, &mut us, &mut draws, self.last_draws.0);

            // The movable props, once their textures are in, with the world's light and fog.
            if let Some(props) = &self.props {
                if props.uploaded == props.pending.len() && !frame.props.is_empty() {
                    let (horizon, scale) = self
                        .district
                        .as_ref()
                        .map(|d| (d.horizon(), d.scale()))
                        .or(self.park_gpu.as_ref().map(|p| (p.horizon, p.scale)))
                        .unwrap_or((SKY, props.scale));
                    let (submitted, culled) = draw_props(props, frame.props, self.white, &self.prop, &self.prop_mask, frame, &vp, horizon, scale);
                    self.last_draws.0 += submitted;
                    self.last_draws.1 += culled;
                    draws[1] += submitted;
                }
            }
            mark(1, &mut at, &mut us, &mut draws, 0);

            // The cars' bodies (opaque), then the glass after the skater.
            let visible_cars = self.draw_cars(frame, &vp, false);
            self.last_draws.0 += visible_cars;
            mark(2, &mut at, &mut us, &mut draws, visible_cars);

            // The peds.
            let (drawn, culled) = self.draw_peds(frame.peds, &self.peds, &frame.view_proj, &vp);
            self.last_draws.0 += drawn;
            self.last_draws.1 += culled;
            // The hand props (held at a ped's hand, or released bodies).
            let (hand_drawn, culled) = self.draw_peds(frame.hand_props, &self.hand_props, &frame.view_proj, &vp);
            self.last_draws.0 += hand_drawn;
            self.last_draws.1 += culled;
            mark(3, &mut at, &mut us, &mut draws, drawn + hand_drawn);

            // Skater and board, then the NPC skaters (the same mesh, their own palettes).
            if self.skater.is_some() {
                self.draw_skin(frame.palette, 1.0, &vp);
                let planes = frustum(&frame.view_proj);
                for npc in frame.npcs {
                    let c = npc.center + Vec3::Y;
                    if planes.iter().all(|p| p.truncate().dot(c) + p.w >= -2.0 * p.truncate().length()) {
                        self.draw_skin(npc.palette, npc.alpha, &vp);
                        self.last_draws.0 += 1;
                    } else {
                        self.last_draws.1 += 1;
                    }
                }
            }
            self.draw_cars(frame, &vp, true);
            // The teleporter icons, blended over everything.
            self.draw_icons(frame.icons, frame.trails, &vp);
            mark(4, &mut at, &mut us, &mut draws, 1 + frame.npcs.len() as u32);
            for i in 0..5 {
                self.pass_us[i] += us[i];
                self.pass_draws[i] += u64::from(draws[i]);
            }
            self.pass_frames += 1;
        }
    }

    /// The scene draw's passes since the last call, per frame: (ms, draws) for the world, the props, the cars, the
    /// peds and hand props, and the skaters with the rest (diagnostics).
    pub fn take_pass_times(&mut self) -> Option<[(f32, f32); 5]> {
        let frames = std::mem::take(&mut self.pass_frames);
        let (us, draws) = (std::mem::take(&mut self.pass_us), std::mem::take(&mut self.pass_draws));
        (frames > 0).then(|| std::array::from_fn(|i| (us[i] as f32 / 1000.0 / frames as f32, draws[i] as f32 / frames as f32)))
    }

    /// The skater mesh with one palette: opaque materials first, masked last; below
    /// opacity 1 (an NPC fading) every material blends at that opacity.
    fn draw_skin(&self, skin_palette: &[Mat4], alpha: f32, vp: &[f32; 16]) {
        let Some(sk) = &self.skater else { return };
        if skin_palette.is_empty() || alpha <= 0.0 {
            return;
        }
        let fading = alpha < 1.0;
        unsafe {
            let joints = skin_palette.len().min(MAX_JOINTS);
            let mut palette = [0f32; 16 * MAX_JOINTS];
            for (slot, m) in palette.chunks_mut(16).zip(&skin_palette[..joints]) {
                slot.copy_from_slice(&m.to_cols_array());
            }
            glBindBuffer(GL_ARRAY_BUFFER, sk.vbo);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, sk.ibo);
            attributes(&[
                (3, GL_FLOAT, GL_FALSE, skater::POSITION),
                (3, GL_BYTE, GL_TRUE, skater::NORMAL),
                (2, GL_FLOAT, GL_FALSE, skater::UV),
                (4, GL_UNSIGNED_BYTE, GL_FALSE, skater::JOINTS),
                (4, GL_UNSIGNED_BYTE, GL_TRUE, skater::WEIGHTS),
            ], skater::STRIDE);
            glActiveTexture(GL_TEXTURE0);
            if fading {
                glEnable(GL_BLEND);
                glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
                let p = &self.skin_fade;
                glUseProgram(p.program.id);
                glUniformMatrix4fv(p.viewproj, 1, GL_FALSE, vp.as_ptr());
                glUniformMatrix4fv(p.bones, MAX_JOINTS as GLsizei, GL_FALSE, palette.as_ptr());
                glUniform1f(self.skin_fade_alpha, alpha);
            }
            let mut current: Option<bool> = None;
            for d in &sk.draws {
                let masked = d.cutoff.is_some();
                let p = if fading { &self.skin_fade } else if masked { &self.skin_mask } else { &self.skin };
                if !fading && current != Some(masked) {
                    glUseProgram(p.program.id);
                    glUniformMatrix4fv(p.viewproj, 1, GL_FALSE, vp.as_ptr());
                    glUniformMatrix4fv(p.bones, MAX_JOINTS as GLsizei, GL_FALSE, palette.as_ptr());
                    current = Some(masked);
                }
                if let Some(location) = p.cutoff {
                    glUniform1f(location, d.cutoff.unwrap_or(if fading { -1.0 } else { 0.0 }));
                }
                glUniform4f(p.tint, d.tint[0], d.tint[1], d.tint[2], d.tint[3]);
                if d.double_sided {
                    glDisable(GL_CULL_FACE);
                } else {
                    glEnable(GL_CULL_FACE);
                }
                glBindTexture(GL_TEXTURE_2D, d.texture);
                glDrawElements(
                    GL_TRIANGLES,
                    d.index_count as GLsizei,
                    GL_UNSIGNED_SHORT,
                    (d.first_index as usize * 2) as *const c_void,
                );
            }
            if fading {
                glDisable(GL_BLEND);
            }
            glDisable(GL_CULL_FACE);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        }
    }

    /// One pass over the cars in view: the bodies (painted, opaque) or the glass (blended,
    /// no depth write). Each car's palette is its transform x the joint's pose (a wheel spun
    /// about its x axis) x the inverse bind. Returns the cars drawn.
    fn draw_cars(&self, frame: &Frame, vp: &[f32; 16], glass: bool) -> u32 {
        if frame.cars.is_empty() || self.cars.is_empty() {
            return 0;
        }
        let planes = frustum(&frame.view_proj);
        let p = if glass { &self.car_glass } else { &self.car };
        let mut drawn = 0;
        unsafe {
            glUseProgram(p.program.id);
            glUniformMatrix4fv(p.viewproj, 1, GL_FALSE, vp.as_ptr());
            if glass {
                glEnable(GL_BLEND);
                glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
                glDepthMask(GL_FALSE);
            }
            glActiveTexture(GL_TEXTURE0);
            let mut palette = [0f32; 16 * MAX_JOINTS];
            for car in frame.cars {
                let Some(Some(gpu)) = self.cars.get(usize::from(car.model)) else { continue };
                let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for corner in 0..8 {
                    let c = Vec3::new(if corner & 1 == 0 { gpu.min.x } else { gpu.max.x }, if corner & 2 == 0 { gpu.min.y } else { gpu.max.y }, if corner & 4 == 0 { gpu.min.z } else { gpu.max.z });
                    let w = car.transform.transform_point3(c);
                    min = min.min(w);
                    max = max.max(w);
                }
                if !visible(&planes, min, max) {
                    continue;
                }
                let joints = gpu.joints.len().min(MAX_JOINTS);
                let spin = Mat4::from_rotation_x(car.wheel);
                for (slot, &(wheel, bind, inverse_bind)) in palette.chunks_mut(16).zip(&gpu.joints[..joints]) {
                    let m = if wheel { car.transform * bind * spin * inverse_bind } else { car.transform };
                    slot.copy_from_slice(&m.to_cols_array());
                }
                glUniformMatrix4fv(p.bones, joints as GLsizei, GL_FALSE, palette.as_ptr());
                if let Some((red, blue)) = p.paint {
                    glUniform4f(red, car.secondary[0], car.secondary[1], car.secondary[2], car.secondary[3]);
                    glUniform4f(blue, car.chassis[0], car.chassis[1], car.chassis[2], car.chassis[3]);
                }
                glBindBuffer(GL_ARRAY_BUFFER, gpu.vbo);
                glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, gpu.ibo);
                attributes(&[
                    (3, GL_FLOAT, GL_FALSE, skater::POSITION),
                    (3, GL_BYTE, GL_TRUE, skater::NORMAL),
                    (2, GL_FLOAT, GL_FALSE, skater::UV),
                    (4, GL_UNSIGNED_BYTE, GL_FALSE, skater::JOINTS),
                    (4, GL_UNSIGNED_BYTE, GL_TRUE, skater::WEIGHTS),
                ], skater::STRIDE);
                for (d, blended) in &gpu.draws {
                    if *blended != glass {
                        continue;
                    }
                    glUniform4f(p.tint, d.tint[0], d.tint[1], d.tint[2], d.tint[3]);
                    if d.double_sided || glass {
                        glDisable(GL_CULL_FACE);
                    } else {
                        glEnable(GL_CULL_FACE);
                    }
                    glBindTexture(GL_TEXTURE_2D, d.texture);
                    glDrawElements(GL_TRIANGLES, d.index_count as GLsizei, GL_UNSIGNED_SHORT, (d.first_index as usize * 2) as *const c_void);
                }
                drawn += 1;
            }
            if glass {
                glDisable(GL_BLEND);
                glDepthMask(GL_TRUE);
            }
            glDisable(GL_CULL_FACE);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        }
        drawn
    }
}

/// Enables exactly these vertex attributes (size, type, normalized, byte offset).
pub(crate) unsafe fn attributes(list: &[(GLint, GLenum, GLboolean, usize)], stride: usize) {
    unsafe {
        for index in 0..6 {
            glDisableVertexAttribArray(index);
        }
        for (index, &(size, kind, normalized, offset)) in list.iter().enumerate() {
            glEnableVertexAttribArray(index as GLuint);
            glVertexAttribPointer(
                index as GLuint,
                size,
                kind,
                normalized,
                stride as GLsizei,
                offset as *const c_void,
            );
        }
    }
}

pub(crate) unsafe fn buffer(target: GLenum, bytes: &[u8]) -> GLuint {
    unsafe {
        let mut id = 0;
        glGenBuffers(1, &mut id);
        glBindBuffer(target, id);
        glBufferData(target, bytes.len() as GLsizei, bytes.as_ptr().cast(), GL_STATIC_DRAW);
        id
    }
}

/// Uploads every level as stored (RGBA8, or DXT1/DXT5 through
/// `glCompressedTexImage2D`), trilinear, repeating.
pub(crate) fn upload_texture(t: &Texture) -> GLuint {
    unsafe {
        let mut id = 0;
        glGenTextures(1, &mut id);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, id);
        let mipmapped = t.mips.len() > 1;
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, if mipmapped { GL_LINEAR_MIPMAP_LINEAR } else { GL_LINEAR });
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT);
        let (mut w, mut h) = (GLsizei::from(t.width), GLsizei::from(t.height));
        for (level, data) in t.mips.iter().enumerate() {
            match t.format {
                Format::Rgba8 => glTexImage2D(GL_TEXTURE_2D, level as GLint, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, data.as_ptr().cast()),
                Format::Dxt1 | Format::Dxt5 => {
                    let format = if t.format == Format::Dxt1 { GL_COMPRESSED_RGBA_S3TC_DXT1_EXT } else { GL_COMPRESSED_RGBA_S3TC_DXT5_EXT };
                    glCompressedTexImage2D(GL_TEXTURE_2D, level as GLint, format, w, h, 0, data.len() as GLsizei, data.as_ptr().cast());
                }
            }
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        id
    }
}

/// Planes of the view frustum (Gribb–Hartmann) for culling draw bounds.
pub(crate) fn frustum(vp: &Mat4) -> [Vec4; 6] {
    let r = [vp.row(0), vp.row(1), vp.row(2), vp.row(3)];
    [r[3] + r[0], r[3] - r[0], r[3] + r[1], r[3] - r[1], r[3] + r[2], r[3] - r[2]]
}

pub(crate) fn visible(planes: &[Vec4; 6], min: Vec3, max: Vec3) -> bool {
    planes.iter().all(|p| {
        let corner = Vec3::new(
            if p.x >= 0.0 { max.x } else { min.x },
            if p.y >= 0.0 { max.y } else { min.y },
            if p.z >= 0.0 { max.z } else { min.z },
        );
        p.truncate().dot(corner) + p.w >= 0.0
    })
}

/// The park's draws (opaque first, then alpha-tested), culled per draw. Returns
/// (submitted, culled).
fn park_draw(d: &park_pack::Draw) -> ParkDraw {
    ParkDraw {
        vertex_offset: d.first_vertex as usize * park_pack::STRIDE,
        first_index: d.first_index,
        index_count: d.index_count,
        albedo: d.albedo.map(usize::from),
        lightmap: d.lightmap.map(usize::from),
        tint: d.color,
        cutoff: d.alpha_cutoff,
        min: Vec3::from_array(d.bounds_min),
        max: Vec3::from_array(d.bounds_max),
    }
}

/// The movable props, each template's draws at the instance's pose (frustum-culled by the
/// template's box moved by the pose). Fog and exposure as the world around them.
#[allow(clippy::too_many_arguments)]
fn draw_props(props: &PropsGpu, instances: &[PropInstance], white: GLuint, opaque: &ParkProgram, masked: &ParkProgram, frame: &Frame, vp: &[f32; 16], horizon: [f32; 3], scale: f32) -> (u32, u32) {
    let planes = frustum(&frame.view_proj);
    let (mut submitted, mut culled) = (0, 0);
    unsafe {
        glDisable(GL_CULL_FACE);
        glBindBuffer(GL_ARRAY_BUFFER, props.vbo);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, props.ibo);
        let mut current: Option<bool> = None;
        for instance in instances {
            let Some((range, lo, hi)) = props.templates.get(usize::from(instance.template)) else { continue };
            let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for corner in 0..8 {
                let p = Vec3::new(if corner & 1 == 0 { lo.x } else { hi.x }, if corner & 2 == 0 { lo.y } else { hi.y }, if corner & 4 == 0 { lo.z } else { hi.z });
                let w = instance.model.transform_point3(p);
                min = min.min(w);
                max = max.max(w);
            }
            if !visible(&planes, min, max) {
                culled += 1;
                continue;
            }
            let model = instance.model.to_cols_array();
            for d in &props.draws[range.clone()] {
                let is_masked = d.cutoff.is_some();
                let p = if is_masked { masked } else { opaque };
                if current != Some(is_masked) {
                    glUseProgram(p.program.id);
                    glUniformMatrix4fv(p.viewproj, 1, GL_FALSE, vp.as_ptr());
                    glUniform3f(p.eye, frame.eye.x, frame.eye.y, frame.eye.z);
                    glUniform2f(p.fog, PARK_FOG[0], PARK_FOG[1]);
                    glUniform3f(p.horizon, horizon[0], horizon[1], horizon[2]);
                    glUniform1f(p.scale, scale);
                    current = Some(is_masked);
                }
                if let Some(location) = p.model {
                    glUniformMatrix4fv(location, 1, GL_FALSE, model.as_ptr());
                }
                if let (Some(location), Some(cutoff)) = (p.cutoff, d.cutoff) {
                    glUniform1f(location, cutoff);
                }
                glUniform4f(p.tint, d.tint[0], d.tint[1], d.tint[2], d.tint[3]);
                glActiveTexture(GL_TEXTURE0);
                glBindTexture(GL_TEXTURE_2D, d.albedo.map_or(white, |t| props.textures[t]));
                glActiveTexture(GL_TEXTURE1);
                glBindTexture(GL_TEXTURE_2D, d.lightmap.map_or(white, |t| props.textures[t]));
                attributes(&[
                    (3, GL_FLOAT, GL_FALSE, d.vertex_offset + park_pack::POSITION),
                    (2, GL_FLOAT, GL_FALSE, d.vertex_offset + park_pack::UV),
                    (2, GL_FLOAT, GL_FALSE, d.vertex_offset + park_pack::LIGHTMAP_UV),
                ], park_pack::STRIDE);
                glDrawElements(GL_TRIANGLES, d.index_count as GLsizei, GL_UNSIGNED_SHORT, (d.first_index as usize * 2) as *const c_void);
                submitted += 1;
            }
        }
        glActiveTexture(GL_TEXTURE0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
    }
    (submitted, culled)
}

fn draw_park(park: &ParkGpu, white: GLuint, opaque: &ParkProgram, masked: &ParkProgram, frame: &Frame, vp: &[f32; 16]) -> (u32, u32) {
    let planes = frustum(&frame.view_proj);
    let (mut submitted, mut culled) = (0, 0);
    unsafe {
        glDisable(GL_CULL_FACE);
        glBindBuffer(GL_ARRAY_BUFFER, park.vbo);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, park.ibo);
        let mut current: Option<bool> = None;
        for d in &park.draws {
            if !visible(&planes, d.min, d.max) {
                culled += 1;
                continue;
            }
            let is_masked = d.cutoff.is_some();
            let p = if is_masked { masked } else { opaque };
            if current != Some(is_masked) {
                glUseProgram(p.program.id);
                glUniformMatrix4fv(p.viewproj, 1, GL_FALSE, vp.as_ptr());
                glUniform3f(p.eye, frame.eye.x, frame.eye.y, frame.eye.z);
                glUniform2f(p.fog, PARK_FOG[0], PARK_FOG[1]);
                glUniform3f(p.horizon, park.horizon[0], park.horizon[1], park.horizon[2]);
                glUniform1f(p.scale, park.scale);
                current = Some(is_masked);
            }
            if let (Some(location), Some(cutoff)) = (p.cutoff, d.cutoff) {
                glUniform1f(location, cutoff);
            }
            glUniform4f(p.tint, d.tint[0], d.tint[1], d.tint[2], d.tint[3]);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, d.albedo.map_or(white, |t| park.textures[t]));
            glActiveTexture(GL_TEXTURE1);
            glBindTexture(GL_TEXTURE_2D, d.lightmap.map_or(white, |t| park.textures[t]));
            attributes(&[
                (3, GL_FLOAT, GL_FALSE, d.vertex_offset + park_pack::POSITION),
                (2, GL_FLOAT, GL_FALSE, d.vertex_offset + park_pack::UV),
                (2, GL_FLOAT, GL_FALSE, d.vertex_offset + park_pack::LIGHTMAP_UV),
            ], park_pack::STRIDE);
            glDrawElements(GL_TRIANGLES, d.index_count as GLsizei, GL_UNSIGNED_SHORT, (d.first_index as usize * 2) as *const c_void);
            submitted += 1;
        }
        glActiveTexture(GL_TEXTURE0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
    }
    (submitted, culled)
}

/// A ped model (or a rigid hand prop model) on the GPU: its buffers, textures and draws with their LOD (materials
/// named `LOD1:` by the bake) and tint flag (`TINT:`).
fn ped_gpu(white: GLuint, s: &Skater) -> PedGpu {
    unsafe {
        let vbo = buffer(GL_ARRAY_BUFFER, &s.vertex_data);
        let index_bytes: Vec<u8> = s.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        let ibo = buffer(GL_ELEMENT_ARRAY_BUFFER, &index_bytes);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        let textures: Vec<GLuint> = s.textures.iter().map(upload_texture).collect();
        let draws = s
            .primitives
            .iter()
            .map(|p| {
                let m = &s.materials[usize::from(p.material)];
                let (lod, name) = match m.name.strip_prefix("LOD1:") {
                    Some(rest) => (1, rest),
                    None => (0, m.name.as_str()),
                };
                let draw = Draw {
                    first_index: p.first_index,
                    index_count: p.index_count,
                    texture: m.texture.map_or(white, |t| textures[usize::from(t)]),
                    tint: m.base_color,
                    cutoff: match m.alpha_mode {
                        AlphaMode::Opaque => None,
                        AlphaMode::Mask => Some(m.alpha_cutoff),
                        AlphaMode::Blend => Some(0.5),
                    },
                    double_sided: m.double_sided,
                };
                (draw, lod, name.starts_with("TINT:"))
            })
            .collect();
        PedGpu { vbo, ibo, textures, draws }
    }
}

unsafe fn free_ped_gpu(c: &PedGpu) {
    unsafe {
        glDeleteBuffers(1, &c.vbo);
        glDeleteBuffers(1, &c.ibo);
        glDeleteTextures(c.textures.len() as GLsizei, c.textures.as_ptr());
    }
}

/// A park render's buffers, its textures queued (`upload_pending`).
fn park_gpu(park: ParkRender) -> ParkGpu {
    unsafe {
        let vbo = buffer(GL_ARRAY_BUFFER, &park.vertex_data);
        let index_bytes: Vec<u8> = park.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        let ibo = buffer(GL_ELEMENT_ARRAY_BUFFER, &index_bytes);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        let draws = park.draws.iter().map(park_draw).collect();
        ParkGpu {
            vbo,
            ibo,
            textures: vec![0; park.textures.len()],
            pending: park.textures.into_iter().map(Some).collect(),
            uploaded: 0,
            draws,
            horizon: park.horizon.map(|c| c.max(0.0).powf(1.0 / 2.2)),
            scale: KD * park.exposure,
        }
    }
}

fn delete_park(p: ParkGpu) {
    unsafe {
        glDeleteBuffers(1, &p.vbo);
        glDeleteBuffers(1, &p.ibo);
        let uploaded: Vec<GLuint> = p.textures.iter().copied().filter(|&t| t != 0).collect();
        glDeleteTextures(uploaded.len() as GLsizei, uploaded.as_ptr());
    }
}
