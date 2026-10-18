//! Shader programs with a GXP binary cache (don't compile shaders at
//! every launch). The first launch compiles the CG sources with vitaGL's runtime
//! compiler and saves each shader's GXP to `ux0:data/SKATEVITA/shaders/`, keyed by a
//! hash of its source; later launches load the binaries.
use core::ffi::CStr;

use crate::vgl::*;

const CACHE: &str = "ux0:data/SKATEVITA/shaders";

pub(crate) struct Program {
    pub id: GLuint,
}

impl Program {
    /// Links `vertex` + `fragment`; attribute `i` of `attributes` is bound to index `i`.
    pub fn new(name: &str, vertex: &CStr, fragment: &CStr, attributes: &[&CStr]) -> Self {
        unsafe {
            let vs = shader(name, "vert", GL_VERTEX_SHADER, vertex);
            let fs = shader(name, "frag", GL_FRAGMENT_SHADER, fragment);
            let id = glCreateProgram();
            glAttachShader(id, vs);
            glAttachShader(id, fs);
            for (index, attribute) in attributes.iter().enumerate() {
                glBindAttribLocation(id, index as GLuint, attribute.as_ptr());
            }
            glLinkProgram(id);
            let mut ok = 0;
            glGetProgramiv(id, GL_LINK_STATUS, &mut ok);
            if ok == 0 {
                println!("shader program {name}: link failed");
            }
            Self { id }
        }
    }

    pub fn uniform(&self, name: &CStr) -> GLint {
        let location = unsafe { glGetUniformLocation(self.id, name.as_ptr()) };
        if location == -1 {
            println!("uniform {name:?} not found");
        }
        location
    }
}

/// FNV-1a: a stable key for the cache file name.
fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

unsafe fn shader(name: &str, stage: &str, kind: GLenum, source: &CStr) -> GLuint {
    let path = format!("{CACHE}/{name}.{stage}.{:016x}.gxp", hash(source.to_bytes()));
    unsafe {
        let handle = glCreateShader(kind);
        if let Ok(binary) = std::fs::read(&path) {
            // vglGetShaderBinary's format (matrix-uniform table + GXP); glShaderBinary
            // recognizes it.
            glShaderBinary(1, &handle, 0, binary.as_ptr().cast(), binary.len() as GLsizei);
            return handle;
        }
        let ptr = source.as_ptr();
        glShaderSource(handle, 1, &ptr, core::ptr::null());
        glCompileShader(handle);
        let mut ok = 0;
        glGetShaderiv(handle, GL_COMPILE_STATUS, &mut ok);
        if ok == 0 {
            let mut log = [0 as GLchar; 1024];
            let mut len = 0;
            glGetShaderInfoLog(handle, log.len() as GLsizei, &mut len, log.as_mut_ptr());
            let msg = CStr::from_ptr(log.as_ptr()).to_string_lossy();
            println!("shader {name}.{stage} failed to compile: {msg}");
            return handle;
        }
        // vglGetShaderBinary writes without checking the buffer size, so give it
        // far more than any of these programs needs (they are a few KiB).
        let mut binary = vec![0u8; 256 * 1024];
        let mut length = 0;
        vglGetShaderBinary(handle, binary.len() as GLsizei, &mut length, binary.as_mut_ptr().cast());
        if length > 0 && (length as usize) <= binary.len() {
            let _ = std::fs::create_dir_all(CACHE);
            match std::fs::write(&path, &binary[..length as usize]) {
                Ok(()) => println!("shader {name}.{stage}: compiled, cached {} bytes", length),
                Err(e) => println!("shader {name}.{stage}: compiled, not cached: {e}"),
            }
        }
        handle
    }
}
