//! Minimal hand-written vitaGL FFI, declared against the header installed with the
//! library we link (`$VITASDK/arm-vita-eabi/include/vitaGL.h`). That build is newer than
//! the `../../vitaGL` sources and has no `vglEnd`. Only what the renderer uses.
#![allow(non_camel_case_types, non_snake_case, dead_code)]

use core::ffi::{c_char, c_int, c_void};

pub type GLboolean = u8;
pub type GLenum = u32;
pub type GLbitfield = u32;
pub type GLuint = u32;
pub type GLint = i32;
pub type GLsizei = i32;
pub type GLfloat = f32;
pub type GLchar = c_char;

pub const GL_FALSE: GLboolean = 0;
pub const GL_TRUE: GLboolean = 1;
pub const GL_TRIANGLES: GLenum = 0x0004;
pub const GL_FLOAT: GLenum = 0x1406;
pub const GL_COLOR_BUFFER_BIT: GLbitfield = 0x0000_4000;
pub const GL_ARRAY_BUFFER: GLenum = 0x8892;
pub const GL_STATIC_DRAW: GLenum = 0x88E4;
pub const GL_FRAGMENT_SHADER: GLenum = 0x8B30;
pub const GL_VERTEX_SHADER: GLenum = 0x8B31;
pub const GL_COMPILE_STATUS: GLenum = 0x8B81;
pub const GL_LINK_STATUS: GLenum = 0x8B82;
pub const GL_TEXTURE_2D: GLenum = 0x0DE1;
pub const GL_RGBA: GLenum = 0x1908;
pub const GL_UNSIGNED_BYTE: GLenum = 0x1401;
pub const GL_TEXTURE_MAG_FILTER: GLenum = 0x2800;
pub const GL_TEXTURE_MIN_FILTER: GLenum = 0x2801;
pub const GL_NEAREST: GLint = 0x2600;
pub const GL_BLEND: GLenum = 0x0BE2;
pub const GL_SRC_ALPHA: GLenum = 0x0302;
pub const GL_ONE: GLenum = 1;
pub const GL_ONE_MINUS_SRC_ALPHA: GLenum = 0x0303;
pub const GL_STREAM_DRAW: GLenum = 0x88E0;
pub const GL_TEXTURE0: GLenum = 0x84C0;
pub const GL_DEPTH_TEST: GLenum = 0x0B71;
pub const GL_CULL_FACE: GLenum = 0x0B44;
pub const GL_LEQUAL: GLenum = 0x0203;
pub const GL_BACK: GLenum = 0x0405;
pub const GL_CCW: GLenum = 0x0901;
pub const GL_BYTE: GLenum = 0x1400;
pub const GL_UNSIGNED_SHORT: GLenum = 0x1403;
pub const GL_DEPTH_BUFFER_BIT: GLbitfield = 0x0000_0100;
pub const GL_ELEMENT_ARRAY_BUFFER: GLenum = 0x8893;
pub const GL_LINEAR: GLint = 0x2601;
pub const GL_LINEAR_MIPMAP_LINEAR: GLint = 0x2703;
pub const GL_TEXTURE_WRAP_S: GLenum = 0x2802;
pub const GL_TEXTURE_WRAP_T: GLenum = 0x2803;
pub const GL_REPEAT: GLint = 0x2901;
pub const GL_CLAMP_TO_EDGE: GLint = 0x812F;
pub const GL_COMPRESSED_RGBA_S3TC_DXT1_EXT: GLenum = 0x83F1;
pub const GL_COMPRESSED_RGBA_S3TC_DXT5_EXT: GLenum = 0x83F3;
pub const GL_TEXTURE1: GLenum = 0x84C1;

/// `SceGxmMultisampleMode`
pub const SCE_GXM_MULTISAMPLE_NONE: c_int = 0;
pub const SCE_GXM_MULTISAMPLE_2X: c_int = 1;
pub const SCE_GXM_MULTISAMPLE_4X: c_int = 2;

unsafe extern "C" {
    /// Free / total bytes of a vitaGL pool (`vglMemType`: 0 VRAM, 1 RAM, 2 SLOW, 3 BUDGET, 4 newlib).
    pub fn vglMemFree(kind: c_int) -> usize;
    pub fn vglMemTotal(kind: c_int) -> usize;
    /// Returns GL_TRUE if the framebuffer had to fall back to a smaller size, not on failure.
    pub fn vglInitExtended(
        legacy_pool_size: c_int,
        width: c_int,
        height: c_int,
        ram_threshold: c_int,
        msaa: c_int,
    ) -> GLboolean;
    pub fn vglSwapBuffers(has_commondialog: GLboolean);
    pub fn vglWaitVblankStart(enable: GLboolean);

    pub fn glViewport(x: GLint, y: GLint, width: GLsizei, height: GLsizei);
    pub fn glClearColor(r: GLfloat, g: GLfloat, b: GLfloat, a: GLfloat);
    pub fn glClear(mask: GLbitfield);

    pub fn glCreateShader(kind: GLenum) -> GLuint;
    pub fn glShaderSource(
        shader: GLuint,
        count: GLsizei,
        string: *const *const GLchar,
        length: *const GLint,
    );
    pub fn glCompileShader(shader: GLuint);
    pub fn glGetShaderiv(shader: GLuint, pname: GLenum, params: *mut GLint);
    pub fn glGetShaderInfoLog(
        shader: GLuint,
        max_length: GLsizei,
        length: *mut GLsizei,
        info_log: *mut GLchar,
    );
    pub fn glCreateProgram() -> GLuint;
    pub fn glAttachShader(program: GLuint, shader: GLuint);
    pub fn glBindAttribLocation(program: GLuint, index: GLuint, name: *const GLchar);
    pub fn glLinkProgram(program: GLuint);
    pub fn glGetProgramiv(program: GLuint, pname: GLenum, params: *mut GLint);
    pub fn glUseProgram(program: GLuint);
    pub fn glGetUniformLocation(program: GLuint, name: *const GLchar) -> GLint;
    pub fn glUniform1f(location: GLint, v0: GLfloat);
    pub fn glUniform2f(location: GLint, v0: GLfloat, v1: GLfloat);

    pub fn glGenBuffers(n: GLsizei, buffers: *mut GLuint);
    pub fn glBindBuffer(target: GLenum, buffer: GLuint);
    /// vitaGL declares `size` as `GLsizei`, not `GLsizeiptr`.
    pub fn glBufferData(target: GLenum, size: GLsizei, data: *const c_void, usage: GLenum);
    pub fn glEnableVertexAttribArray(index: GLuint);
    pub fn glVertexAttribPointer(
        index: GLuint,
        size: GLint,
        kind: GLenum,
        normalized: GLboolean,
        stride: GLsizei,
        pointer: *const c_void,
    );
    pub fn glDrawArrays(mode: GLenum, first: GLint, count: GLsizei);

    pub fn glGenTextures(n: GLsizei, textures: *mut GLuint);
    pub fn glReadPixels(x: GLint, y: GLint, width: GLsizei, height: GLsizei, format: GLenum, type_: GLenum, data: *mut c_void);
    pub fn glDeleteTextures(n: GLsizei, textures: *const GLuint);
    pub fn glDeleteBuffers(n: GLsizei, buffers: *const GLuint);
    pub fn glBindTexture(target: GLenum, texture: GLuint);
    pub fn glActiveTexture(texture: GLenum);
    pub fn glTexParameteri(target: GLenum, pname: GLenum, param: GLint);
    pub fn glTexImage2D(
        target: GLenum,
        level: GLint,
        internal_format: GLint,
        width: GLsizei,
        height: GLsizei,
        border: GLint,
        format: GLenum,
        kind: GLenum,
        data: *const c_void,
    );
    pub fn glUniform1i(location: GLint, v0: GLint);
    pub fn glEnable(cap: GLenum);
    pub fn glDisable(cap: GLenum);
    pub fn glBlendFunc(sfactor: GLenum, dfactor: GLenum);
    pub fn glDepthFunc(func: GLenum);
    pub fn glDepthMask(flag: GLboolean);
    pub fn glClearDepthf(depth: GLfloat);
    pub fn glCullFace(mode: GLenum);
    pub fn glFrontFace(mode: GLenum);
    pub fn glDisableVertexAttribArray(index: GLuint);
    pub fn glDrawElements(mode: GLenum, count: GLsizei, kind: GLenum, indices: *const c_void);
    pub fn glGenerateMipmap(target: GLenum);
    pub fn glCompressedTexImage2D(
        target: GLenum,
        level: GLint,
        internal_format: GLenum,
        width: GLsizei,
        height: GLsizei,
        border: GLint,
        image_size: GLsizei,
        data: *const c_void,
    );
    pub fn glUniform3f(location: GLint, v0: GLfloat, v1: GLfloat, v2: GLfloat);
    pub fn glUniform4f(location: GLint, v0: GLfloat, v1: GLfloat, v2: GLfloat, v3: GLfloat);
    pub fn glUniform4fv(location: GLint, count: GLsizei, value: *const GLfloat);
    pub fn glUniformMatrix4fv(location: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat);
    /// Loads a GXP, or vitaGL's (matrix-uniform table + GXP) from `vglGetShaderBinary`.
    pub fn glShaderBinary(
        count: GLsizei,
        handles: *const GLuint,
        binary_format: GLenum,
        binary: *const c_void,
        length: GLsizei,
    );
    /// The compiled shader in vitaGL's binary format. Does not check `buf_size`.
    pub fn vglGetShaderBinary(handle: GLuint, buf_size: GLsizei, length: *mut GLsizei, binary: *mut c_void);
}
