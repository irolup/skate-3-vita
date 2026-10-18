//! The retail HUD: draws the batches of the original APT movie
//! (`skate_ui::apt_scene::draw`): triangles with texture coordinates in the movie's
//! 1280 × 720 stage space, each batch with a multiply/add colour transform. The stage is
//! scaled to the 960 × 544 screen (0.75, centred vertically). Plain alpha blending onto
//! the frame equals the reference's offscreen target plus premultiplied composite
//! ("over" is associative); only the blend space differs (the Vita's framebuffer is not
//! sRGB, so soft glow edges are blended in gamma space).
use core::ffi::{CStr, c_void};
use std::collections::BTreeMap;

use skate_ui::apt_scene::Draw;

use crate::math::{HEIGHT, WIDTH};
use crate::vgl::*;

const VERT_SRC: &CStr = cstr(concat!(include_str!("../../../shaders/apt.vert.cg"), "\0"));
const FRAG_SRC: &CStr = cstr(concat!(include_str!("../../../shaders/apt.frag.cg"), "\0"));

const fn cstr(s: &str) -> &CStr {
    match CStr::from_bytes_with_nul(s.as_bytes()) {
        Ok(c) => c,
        Err(_) => panic!("shader source must not contain NUL"),
    }
}

/// The movie's stage.
const STAGE: (f32, f32) = (1280.0, 720.0);
/// x, y, u, v
const FLOATS: usize = 4;

pub struct Apt {
    program: crate::gl::Program,
    vbo: GLuint,
    textures: BTreeMap<String, GLuint>,
    verts: Vec<f32>,
    multiply: GLint,
    add: GLint,
}

impl Apt {
    /// Call after [`crate::init`].
    pub fn new() -> Self {
        let program = crate::gl::Program::new("apt", VERT_SRC, FRAG_SRC, &[c"aPosition", c"aTexcoord"]);
        unsafe {
            glUseProgram(program.id);
            let scale = WIDTH as f32 / STAGE.0;
            glUniform3f(program.uniform(c"stage"), scale, 0.0, (HEIGHT as f32 - STAGE.1 * scale) / 2.0);
            glUniform2f(program.uniform(c"screen"), WIDTH as f32, HEIGHT as f32);
            glUniform1i(program.uniform(c"atlas"), 0);
            let mut vbo = 0;
            glGenBuffers(1, &mut vbo);
            let (multiply, add) = (program.uniform(c"colorMultiply"), program.uniform(c"colorAdd"));
            let mut apt = Self { program, vbo, textures: BTreeMap::new(), verts: Vec::with_capacity(16 * 1024), multiply, add };
            // Untextured fills of the movies (`skate_ui::apt_scene::WHITE`).
            apt.add_texture("white", 1, 1, &[255; 4]);
            apt
        }
    }

    /// Uploads a straight-alpha RGBA8 texture under the movie's key (its file name).
    pub fn add_texture(&mut self, key: &str, width: u32, height: u32, rgba: &[u8]) {
        assert_eq!(rgba.len(), width as usize * height as usize * 4);
        unsafe {
            let mut id = 0;
            glGenTextures(1, &mut id);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, id);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, width as GLsizei, height as GLsizei, 0, GL_RGBA, GL_UNSIGNED_BYTE, rgba.as_ptr().cast());
            if let Some(old) = self.textures.insert(key.to_owned(), id) {
                glDeleteTextures(1, &old);
            }
        }
    }

    pub fn has_texture(&self, key: &str) -> bool {
        self.textures.contains_key(key)
    }

    /// Draws one frame of the movie over whatever is on screen (no clear, no swap).
    /// Batches whose texture was not uploaded are skipped.
    pub fn draw(&mut self, draws: &[Draw]) {
        self.verts.clear();
        let mut batches: Vec<(GLuint, GLint, GLsizei, &Draw)> = Vec::with_capacity(draws.len());
        for d in draws {
            let Some(&texture) = self.textures.get(&d.texture) else { continue };
            let first = (self.verts.len() / FLOATS) as GLint;
            for v in &d.vertices {
                self.verts.extend_from_slice(&[v.position[0], v.position[1], v.uv[0], v.uv[1]]);
            }
            batches.push((texture, first, d.vertices.len() as GLsizei, d));
        }
        if batches.is_empty() {
            return;
        }
        unsafe {
            glDisable(GL_DEPTH_TEST);
            glDisable(GL_CULL_FACE);
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            glUseProgram(self.program.id);
            glActiveTexture(GL_TEXTURE0);
            glBindBuffer(GL_ARRAY_BUFFER, self.vbo);
            glBufferData(GL_ARRAY_BUFFER, (self.verts.len() * size_of::<f32>()) as GLsizei, self.verts.as_ptr().cast(), GL_STREAM_DRAW);
            let stride = (FLOATS * size_of::<f32>()) as GLsizei;
            for index in 0..6 {
                glDisableVertexAttribArray(index);
            }
            for (index, offset) in [0usize, 2].into_iter().enumerate() {
                glEnableVertexAttribArray(index as GLuint);
                glVertexAttribPointer(index as GLuint, 2, GL_FLOAT, GL_FALSE, stride, (offset * size_of::<f32>()) as *const c_void);
            }
            for (texture, first, count, d) in batches {
                glBindTexture(GL_TEXTURE_2D, texture);
                glUniform4f(self.multiply, d.multiply[0], d.multiply[1], d.multiply[2], d.multiply[3]);
                glUniform4f(self.add, d.add[0], d.add[1], d.add[2], d.add[3]);
                glDrawArrays(GL_TRIANGLES, first, count);
            }
            glDisable(GL_BLEND);
        }
    }
}

const DISSOLVE_VERT: &CStr = cstr(concat!(include_str!("../../../shaders/dissolve.vert.cg"), "\0"));
const DISSOLVE_FRAG: &CStr = cstr(concat!(include_str!("../../../shaders/dissolve.frag.cg"), "\0"));

/// The session-marker return dissolve: a screen-wide noise pass whose
/// alpha is the hold progress. The noise texture is the reference's native generator
/// output (64 x 64 RGBA, repeating).
pub struct Dissolve {
    program: crate::gl::Program,
    vbo: GLuint,
    texture: GLuint,
    scroll: GLint,
    first: GLint,
    second: GLint,
    fade: GLint,
}

impl Dissolve {
    pub fn new(noise_rgba: &[u8]) -> Self {
        assert_eq!(noise_rgba.len(), 64 * 64 * 4);
        let program = crate::gl::Program::new("dissolve", DISSOLVE_VERT, DISSOLVE_FRAG, &[c"aPosition", c"aTexcoord"]);
        unsafe {
            glUseProgram(program.id);
            glUniform1i(program.uniform(c"noise"), 0);
            let mut texture = 0;
            glGenTextures(1, &mut texture);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, texture);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 64, 64, 0, GL_RGBA, GL_UNSIGNED_BYTE, noise_rgba.as_ptr().cast());
            // x, y, u, v for a full-screen triangle pair.
            let quad: [f32; 24] = [
                -1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0,
                -1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 0.0,
            ];
            let mut vbo = 0;
            glGenBuffers(1, &mut vbo);
            glBindBuffer(GL_ARRAY_BUFFER, vbo);
            glBufferData(GL_ARRAY_BUFFER, size_of_val(&quad) as GLsizei, quad.as_ptr().cast(), GL_STATIC_DRAW);
            let (scroll, first, second, fade) = (program.uniform(c"scroll"), program.uniform(c"firstWeights"), program.uniform(c"secondWeights"), program.uniform(c"fade"));
            Self { program, vbo, texture, scroll, first, second, fade }
        }
    }

    /// Draws the pass over the frame with the generator's current parameters.
    pub fn draw(&mut self, scroll: [f32; 4], first: [f32; 4], second: [f32; 4], progress: f32) {
        unsafe {
            glDisable(GL_DEPTH_TEST);
            glDisable(GL_CULL_FACE);
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            glUseProgram(self.program.id);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, self.texture);
            glUniform4f(self.scroll, scroll[0], scroll[1], scroll[2], scroll[3]);
            glUniform4f(self.first, first[0], first[1], first[2], first[3]);
            glUniform4f(self.second, second[0], second[1], second[2], second[3]);
            glUniform1f(self.fade, progress);
            glBindBuffer(GL_ARRAY_BUFFER, self.vbo);
            for index in 0..6 {
                glDisableVertexAttribArray(index);
            }
            let stride = (4 * size_of::<f32>()) as GLsizei;
            for (index, offset) in [0usize, 2].into_iter().enumerate() {
                glEnableVertexAttribArray(index as GLuint);
                glVertexAttribPointer(index as GLuint, 2, GL_FLOAT, GL_FALSE, stride, (offset * size_of::<f32>()) as *const c_void);
            }
            glDrawArrays(GL_TRIANGLES, 0, 6);
            glDisable(GL_BLEND);
        }
    }
}
