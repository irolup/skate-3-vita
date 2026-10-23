//! 2D UI layer (debug screens, HUD): one shader, one RGBA atlas (the 128 glyphs of
//! font8x8's public-domain 8×8 font; cell 0 is solid white for plain quads) and a
//! vertex batch rebuilt every frame. Started as the calibration screen.
use core::ffi::{CStr, c_void};

use crate::vgl::*;

pub use crate::math::{HEIGHT, WIDTH};

const VERT_SRC: &CStr = cstr(concat!(include_str!("../../../shaders/ui.vert.cg"), "\0"));
const FRAG_SRC: &CStr = cstr(concat!(include_str!("../../../shaders/ui.frag.cg"), "\0"));

const fn cstr(s: &str) -> &CStr {
    match CStr::from_bytes_with_nul(s.as_bytes()) {
        Ok(c) => c,
        Err(_) => panic!("shader source must not contain NUL"),
    }
}

/// Atlas: 16 × 8 cells of 8 × 8 pixels.
const ATLAS_W: usize = 128;
const ATLAS_H: usize = 64;
/// x, y, u, v, r, g, b, a
const FLOATS: usize = 8;

pub type Color = [f32; 4];
pub const WHITE: Color = [1.0, 1.0, 1.0, 1.0];
pub const GREY: Color = [0.55, 0.58, 0.62, 1.0];
pub const DIM: Color = [0.25, 0.28, 0.32, 1.0];
pub const GREEN: Color = [0.35, 0.95, 0.45, 1.0];
pub const YELLOW: Color = [1.0, 0.85, 0.25, 1.0];
pub const RED: Color = [1.0, 0.35, 0.3, 1.0];
pub const CYAN: Color = [0.3, 0.85, 1.0, 1.0];

pub struct Gfx {
    program: GLuint,
    atlas: GLuint,
    vbo: GLuint,
    verts: Vec<f32>,
}

impl Gfx {
    /// UI resources; call after [`crate::init`].
    pub fn new() -> Self {
        unsafe {
            let program = crate::gl::Program::new("ui", VERT_SRC, FRAG_SRC, &[c"aPosition", c"aTexcoord", c"aColor"]);
            glUseProgram(program.id);
            glUniform2f(program.uniform(c"screen"), WIDTH as f32, HEIGHT as f32);
            glUniform1i(program.uniform(c"atlas"), 0);

            let mut atlas_id = 0;
            glGenTextures(1, &mut atlas_id);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, atlas_id);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
            let atlas = atlas();
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as GLint,
                ATLAS_W as GLsizei,
                ATLAS_H as GLsizei,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                atlas.as_ptr().cast(),
            );

            let mut vbo = 0;
            glGenBuffers(1, &mut vbo);
            Self {
                program: program.id,
                atlas: atlas_id,
                vbo,
                verts: Vec::with_capacity(64 * 1024),
            }
        }
    }

    fn vertex(&mut self, x: f32, y: f32, u: f32, v: f32, c: Color) {
        self.verts.extend_from_slice(&[x, y, u, v, c[0], c[1], c[2], c[3]]);
    }

    /// A quad from four corners (clockwise or not) with one atlas cell's corners.
    fn quad(&mut self, p: [[f32; 2]; 4], uv: [f32; 4], c: Color) {
        let [u0, v0, u1, v1] = uv;
        let t = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
        for i in [0, 1, 2, 0, 2, 3] {
            self.vertex(p[i][0], p[i][1], t[i][0], t[i][1], c);
        }
    }

    /// The solid cell's centre: plain colour.
    const SOLID: [f32; 4] = [2.0 / 128.0, 2.0 / 64.0, 6.0 / 128.0, 6.0 / 64.0];

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Color) {
        self.quad([[x, y], [x + w, y], [x + w, y + h], [x, y + h]], Self::SOLID, c);
    }

    pub fn line(&mut self, a: [f32; 2], b: [f32; 2], width: f32, c: Color) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (nx, ny) = (-dy / len * width * 0.5, dx / len * width * 0.5);
        self.quad(
            [[a[0] + nx, a[1] + ny], [b[0] + nx, b[1] + ny], [b[0] - nx, b[1] - ny], [a[0] - nx, a[1] - ny]],
            Self::SOLID,
            c,
        );
    }

    pub fn circle(&mut self, centre: [f32; 2], radius: f32, width: f32, c: Color) {
        let n = 48;
        for i in 0..n {
            let a = i as f32 / n as f32 * core::f32::consts::TAU;
            let b = (i + 1) as f32 / n as f32 * core::f32::consts::TAU;
            self.line(
                [centre[0] + radius * a.cos(), centre[1] + radius * a.sin()],
                [centre[0] + radius * b.cos(), centre[1] + radius * b.sin()],
                width,
                c,
            );
        }
    }

    /// Text in the 8×8 font at an integer scale; returns the x after the last glyph.
    pub fn text(&mut self, x: f32, y: f32, scale: f32, c: Color, s: &str) -> f32 {
        let size = 8.0 * scale;
        let mut pen = x;
        for ch in s.chars() {
            let code = if (ch as u32) < 128 { ch as usize } else { b'?' as usize };
            if code != b' ' as usize {
                let (col, row) = ((code % 16) as f32, (code / 16) as f32);
                let uv = [col * 8.0 / 128.0, row * 8.0 / 64.0, (col + 1.0) * 8.0 / 128.0, (row + 1.0) * 8.0 / 64.0];
                self.quad([[pen, y], [pen + size, y], [pen + size, y + size], [pen, y + size]], uv, c);
            }
            pen += size;
        }
        pen
    }

    /// Draws the batched UI over whatever is on screen (no clear, no swap).
    pub fn draw(&mut self) {
        unsafe {
            glDisable(GL_DEPTH_TEST);
            glDisable(GL_CULL_FACE);
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            if !self.verts.is_empty() {
                glUseProgram(self.program);
                glActiveTexture(GL_TEXTURE0);
                glBindTexture(GL_TEXTURE_2D, self.atlas);
                glBindBuffer(GL_ARRAY_BUFFER, self.vbo);
                glBufferData(
                    GL_ARRAY_BUFFER,
                    (self.verts.len() * size_of::<f32>()) as GLsizei,
                    self.verts.as_ptr().cast(),
                    GL_STREAM_DRAW,
                );
                let stride = (FLOATS * size_of::<f32>()) as GLsizei;
                for index in 0..6 {
                    glDisableVertexAttribArray(index);
                }
                for (index, (size, offset)) in [(2, 0), (2, 2), (4, 4)].into_iter().enumerate() {
                    glEnableVertexAttribArray(index as GLuint);
                    glVertexAttribPointer(
                        index as GLuint,
                        size,
                        GL_FLOAT,
                        GL_FALSE,
                        stride,
                        (offset * size_of::<f32>()) as *const c_void,
                    );
                }
                glDrawArrays(GL_TRIANGLES, 0, (self.verts.len() / FLOATS) as GLsizei);
            }
            glDisable(GL_BLEND);
        }
        self.verts.clear();
    }

    /// A full UI frame: clear, draw, swap (the input lab's screen).
    pub fn present(&mut self) {
        unsafe {
            glClearColor(0.06, 0.07, 0.09, 1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        }
        self.draw();
        crate::swap(false);
    }
}

/// The glyph atlas: font8x8 row bytes have bit 0 as the leftmost pixel.
fn atlas() -> Vec<u8> {
    let mut pixels = vec![0u8; ATLAS_W * ATLAS_H * 4];
    for (code, glyph) in font8x8::legacy::BASIC_LEGACY.iter().enumerate() {
        let (cx, cy) = ((code % 16) * 8, (code / 16) * 8);
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..8 {
                let on = code == 0 || bits & (1 << col) != 0;
                let i = ((cy + row) * ATLAS_W + cx + col) * 4;
                pixels[i..i + 4].copy_from_slice(&[255, 255, 255, if on { 255 } else { 0 }]);
            }
        }
    }
    pixels
}
