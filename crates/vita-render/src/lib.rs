//! skate vita renderer on vitaGL. vitaGL owns the display; there is
//! no SDL. The pure parts (projection, the Course mesh) build and are tested on the
//! desktop; the GL parts are Vita-only.
pub mod course;
pub mod math;
#[cfg(target_os = "vita")]
pub mod apt;
#[cfg(target_os = "vita")]
pub mod district;
#[cfg(target_os = "vita")]
mod gl;
#[cfg(target_os = "vita")]
pub mod scene;
#[cfg(target_os = "vita")]
pub mod ui;
#[cfg(target_os = "vita")]
pub mod vgl;

/// MAIN memory vitaGL leaves free for everything else.
#[cfg(target_os = "vita")]
const RAM_THRESHOLD: i32 = 16 * 1024 * 1024;

/// Initializes vitaGL (960 × 544, no MSAA, vsync on). Call once before anything else.
#[cfg(target_os = "vita")]
pub fn init() {
    use vgl::*;
    unsafe {
        // Not a success flag: GL_TRUE means a smaller framebuffer than requested.
        if vglInitExtended(0, math::WIDTH, math::HEIGHT, RAM_THRESHOLD, SCE_GXM_MULTISAMPLE_NONE) != GL_FALSE {
            println!("vglInitExtended: fell back to a smaller framebuffer than {}x{}", math::WIDTH, math::HEIGHT);
        }
        vglWaitVblankStart(GL_TRUE);
        glViewport(0, 0, math::WIDTH, math::HEIGHT);
    }
}

/// Shows the frame (waits for vblank); `dialog`: a system dialog (the keyboard) runs and draws over it.
#[cfg(target_os = "vita")]
pub fn swap(dialog: bool) {
    unsafe { vgl::vglSwapBuffers(if dialog { vgl::GL_TRUE } else { vgl::GL_FALSE }) };
}

#[cfg(target_os = "vita")]
/// The frame buffer as RGBA8, opaque (alpha 255), bottom row first (GL's order): `WIDTH` x `HEIGHT`.
pub fn read_rgba() -> Vec<u8> {
    use vgl::*;
    let (w, h) = (math::WIDTH as usize, math::HEIGHT as usize);
    let mut rgba = vec![0u8; w * h * 4];
    unsafe { glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, rgba.as_mut_ptr().cast()) };
    for px in rgba.chunks_exact_mut(4) {
        px[3] = 255;
    }
    rgba
}

#[cfg(target_os = "vita")]
/// The frame buffer as a 24-bit BMP (what is on screen after the last draw, 960 x 544). Rows are
/// kept in the order GL returns them (bottom row first), which is a BMP's own order.
pub fn screenshot_bmp() -> Vec<u8> {
    use vgl::*;
    let (w, h) = (math::WIDTH as usize, math::HEIGHT as usize);
    let mut rgba = vec![0u8; w * h * 4];
    unsafe { glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, rgba.as_mut_ptr().cast()) };
    let row = (w * 3 + 3) & !3;
    let size = 54 + row * h;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in 0..h {
        let line = &rgba[y * w * 4..(y + 1) * w * 4];
        for px in line.chunks_exact(4) {
            out.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        out.resize(54 + (y + 1) * row, 0);
    }
    out
}
