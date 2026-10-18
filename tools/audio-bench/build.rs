//! Vita builds: link tools/vita-fpu/fpscr_ieee.c (every thread in IEEE float mode)
//! and tools/vita-libc/aeabi_mem.c (fast __aeabi_mem*), and keep the stack /
//! sceLibc heap exports (see crates/skate-vita/build.rs).
use std::path::PathBuf;

fn main() {
    let sources = [PathBuf::from("../vita-fpu/fpscr_ieee.c"), PathBuf::from("../vita-libc/aeabi_mem.c")];
    println!("cargo:rerun-if-changed=build.rs");
    for src in &sources {
        println!("cargo:rerun-if-changed={}", src.display());
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("vita") {
        return;
    }
    for src in &sources {
        let obj = PathBuf::from(std::env::var("OUT_DIR").unwrap())
            .join(src.file_stem().unwrap())
            .with_extension("o");
        let status = std::process::Command::new("arm-vita-eabi-gcc")
            .args(["-O2", "-mcpu=cortex-a9", "-mfpu=neon", "-mfloat-abi=hard", "-c"])
            .args(std::env::var_os("CARGO_FEATURE_MEM_COUNT").map(|_| "-DAEABI_COUNT"))
            .arg(src)
            .arg("-o")
            .arg(&obj)
            .status()
            .expect("arm-vita-eabi-gcc (VitaSDK bin must be on PATH)");
        assert!(status.success(), "compiling {}", src.display());
        println!("cargo:rustc-link-arg-bins={}", obj.display());
    }
    // link-args land after the libraries, so pthread is listed again after the object.
    println!("cargo:rustc-link-arg-bins=-lpthread");
    println!("cargo:rustc-link-arg-bins=-Wl,--wrap=pthread_create");
    // SceLibKernel provides sceClibMemcpy/Memset/Memmove.
    println!("cargo:rustc-link-arg-bins=-lSceLibKernel_stub");
    for sym in ["sceUserMainThreadStackSize", "sceLibcHeapSize", "__aeabi_memcpy8"] {
        println!("cargo:rustc-link-arg-bins=-Wl,--require-defined={sym}");
    }
}
