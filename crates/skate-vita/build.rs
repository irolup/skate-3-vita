//! Links vitaGL and its dependencies for Vita builds, plus the two C
//! shims every Vita binary needs: tools/vita-fpu/fpscr_ieee.c (IEEE float mode on every
//! thread) and tools/vita-libc/aeabi_mem.c (fast __aeabi_mem*).
//! The list follows the vitaGL samples' Makefile. SceCtrl_stub and SceLibKernel_stub are
//! left out: the vitasdk-sys features of the same name link them, and each stub is linked
//! in one place only.

const VITAGL_LIBS: &[&str] = &[
    "vitaGL",
    "SceAppMgr_stub",
    "SceAppUtil_stub",
    "mathneon",
    "c",
    "SceCommonDialog_stub",
    "m",
    "SceGxm_stub",
    "SceDisplay_stub",
    "SceSysmodule_stub",
    "vitashark",
    "SceShaccCg_stub",
    "SceKernelDmacMgr_stub",
    "stdc++",
    "toloader",
    "SceShaccCgExt",
    "taihen_stub",
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../shaders");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("vita") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    // newlib's libc.a defines strong __aeabi_mem* too (memset/memmove as byte loops),
    // and vitaGL's C code would pull them in. As an archive listed before vitaGL and
    // libc, the shim resolves every reference first, so newlib's members are never
    // pulled and nothing is defined twice.
    let aeabi = compile(&out, "../../tools/vita-libc/aeabi_mem.c");
    let status = std::process::Command::new("arm-vita-eabi-ar")
        .arg("rcs")
        .arg(out.join("libskate_vita_aeabi.a"))
        .arg(&aeabi)
        .status()
        .expect("arm-vita-eabi-ar");
    assert!(status.success(), "archiving aeabi_mem.o");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=skate_vita_aeabi");
    for lib in VITAGL_LIBS {
        println!("cargo:rustc-link-lib=static={lib}");
    }
    let fpscr = compile(&out, "../../tools/vita-fpu/fpscr_ieee.c");
    println!("cargo:rustc-link-arg-bins={}", fpscr.display());
    // link-args land after the libraries, so pthread is listed again after the objects.
    println!("cargo:rustc-link-arg-bins=-lpthread");
    println!("cargo:rustc-link-arg-bins=-Wl,--wrap=pthread_create");
    // The C code's small mallocs go to an area of their own (src/alloc.rs, c_malloc). Not `_malloc_r` nor `_calloc_r`:
    // newlib's own `_memalign_r` and `_realloc_r` call `_malloc_r` and take its result for a chunk of the main heap.
    for sym in ["malloc", "free", "realloc", "calloc", "malloc_usable_size", "_free_r", "_realloc_r", "_malloc_usable_size_r"] {
        println!("cargo:rustc-link-arg-bins=-Wl,--wrap={sym}");
    }
    // `mem-log`: these pass through, counting who calls them (what stays in the main heap).
    if std::env::var_os("CARGO_FEATURE_MEM_LOG").is_some() {
        for sym in ["_malloc_r", "_calloc_r", "memalign"] {
            println!("cargo:rustc-link-arg-bins=-Wl,--wrap={sym}");
        }
    }
    // vita-elf-create reads these by symbol name, but nothing references them, so
    // --gc-sections drops them (#[used] only survives until link time). Make them roots.
    for sym in ["sceUserMainThreadStackSize", "sceLibcHeapSize", "__aeabi_memcpy8"] {
        println!("cargo:rustc-link-arg-bins=-Wl,--require-defined={sym}");
    }
}

fn compile(out: &std::path::Path, src: &str) -> std::path::PathBuf {
    println!("cargo:rerun-if-changed={src}");
    let src = std::path::Path::new(src);
    let obj = out.join(src.file_stem().unwrap()).with_extension("o");
    let status = std::process::Command::new("arm-vita-eabi-gcc")
        .args(["-O2", "-mcpu=cortex-a9", "-mfpu=neon", "-mfloat-abi=hard", "-c"])
        .arg(src)
        .arg("-o")
        .arg(&obj)
        .status()
        .expect("arm-vita-eabi-gcc (VitaSDK bin must be on PATH)");
    assert!(status.success(), "compiling {}", src.display());
    obj
}
