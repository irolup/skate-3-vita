# Toolchain

How the build machine is set up. Verified on Linux Mint (kernel 6.8).

## Installed

| Component | Version | How |
|---|---|---|
| VitaSDK | build of 2025-07-20, at `$VITASDK=~/vitasdk` | pre-installed; provides `libvitaGL.a` (vitaGL `fdf4184`), `libvitashark.a`, `libmathneon.a` |
| rustup | 1.29.1 | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh -s -- -y --default-toolchain nightly -c rust-src` |
| Rust | `rustc 1.100.0-nightly (5ceaf6608 2026-09-25)`, LLVM 23.1.1 | as above, with `rust-src` |
| cargo-vita | 0.2.2 (`c6bbf22`) **plus `docs/patches/cargo-vita-binary-ftp.patch`** | first `cargo +nightly install cargo-vita`; replaced by the patched build, see below |

`rust-toolchain.toml` selects `nightly` with `rust-src`. The Vita target `armv7-sony-vita-newlibeabihf` is tier 3; cargo-vita passes `-Z build-std` using `build_std` from `[package.metadata.vita]`.

## Target facts worth knowing

- The built-in target spec already sets `cpu = "cortex-a9"` and `+v7,+neon,+vfp3,+thumb2,+thumb-mode`, so `-C target-cpu=cortex-a9` is the default.
- cargo-vita sets `RUSTFLAGS` itself (it appends `--cfg mio_unsupported_force_poll_poll ...` to the existing `RUSTFLAGS`). An environment `RUSTFLAGS` overrides `rustflags` from `.cargo/config.toml`, so codegen flags for the Vita must go in the environment `RUSTFLAGS`, not in the config file.
- **cargo-vita strips away the main-thread stack size.** vita-elf-create reads `sceUserMainThreadStackSize` and `sceLibcHeapSize` by symbol name. Two things remove them: `--gc-sections` (nothing references them; `#[used]` only lasts until link time), and cargo-vita's `strip --strip-unneeded`, which runs *before* vita-elf-create. Only `_newlib_heap_size_user` survives both, because newlib's `sbrk` references it. `crates/skate-vita` fixes this with `-Wl,--require-defined=<sym>` in `build.rs` and `strip_symbols = false` under `[package.metadata.vita.profile.release]`. At startup the hello logs the real main-thread stack size to confirm it.
- **Vita codegen flag: `-C llvm-args=-pre-RA-sched=list-ilp`.** LLVM's default pre-RA scheduler for this target hoists loads so far that the solver kernels spill heavily (only 32 single-precision registers): 223 spill stores in `packed::solve`, against 85 with the ILP list scheduler, and the 4 heavy scenarios run about 10% faster (M3). It only changes instruction order and register allocation; the Vita dumps stay bit-identical. `scripts/vita-test.py`'s `sdk_env()` (also used by vita-bench) adds it; for a manual `cargo vita build`, export `RUSTFLAGS="-C llvm-args=-pre-RA-sched=list-ilp"` first (cargo-vita appends its own flags to it).
- **Slow AEABI memory builtins.** Rust's `compiler_builtins` provides `__aeabi_memcpy*`, `__aeabi_memmove*`, `__aeabi_memset*` and `__aeabi_memclr*` as weak one-word loops, and the compiler calls them for every large zero-fill and struct copy (2,000+ call sites in the sim). On the Vita that costs about 0.5 cycles per byte (zeroing 1 KB takes 1.25 µs, copying 1 KB takes 1.7 µs). `tools/vita-libc/aeabi_mem.c` defines them strongly and forwards to the firmware's `sceClibMemcpy`/`Memmove`/`Memset`, which plain `memcpy`/`memset` already use: 468 ns and 915 ns per KB, and about 7% less sim time. Every Vita binary should link it (tools/sim-bench/build.rs does).
- cargo-vita adds `-d ATTRIBUTE2=12` to `vita-mksfoex` by default (365 MiB MAIN budget).

## Smoke tests (step M0.a)

Built from a scratch copy of `vita-rust/examples` (`5ef9e08`) so nothing was written into the reference folder:

| Example | Command | Result |
|---|---|---|
| `0-std-tests` | `cargo vita build vpk -- --release --package vita-std-tests --tests` | VPK, 590 KB. It is a library crate, so its VPK comes from the test harness (`--tests`); a plain build makes no VPK. |
| `4-vitasdk` | `cargo vita build vpk -- --release` | VPK, 621 KB |

The cold `0-std-tests` build (std from source plus dependencies) took 20 s.

## cargo-vita FTP fix (required)

cargo-vita 0.2.2 (crates.io, and the pinned reference `c6bbf22`) never sends `TYPE I`. vitacompanion's FTPVita then transfers in ASCII mode and rewrites `CR LF` → `LF` and `CR NUL` → `CR`. The first VPK upload lost 4 bytes and VitaShell refused it with `0xFFFFFFEC` (-20, libarchive `ARCHIVE_WARN`). `eboot --update` uploads the same way, so it would corrupt `eboot.bin` too. A 10-byte test file confirmed it: `cargo vita upload` changed it, `curl -T` (binary) kept it intact.

The patch switches to binary mode right after connecting. Install it from a scratch copy, so the reference folder stays untouched:

```sh
cp -r ../vita-rust/cargo-vita /tmp/cargo-vita && cd /tmp/cargo-vita
git apply <skate_vita>/docs/patches/cargo-vita-binary-ftp.patch
cargo +nightly install --locked --force --path .
```

After the fix, an uploaded VPK has the same md5 on the Vita as locally. Until upstream fixes it, never upload with an unpatched cargo-vita.

## Vita side (user action)

- vitacompanion: FTP on port 1337, command server on 1338.
- PrincessLog: stdout over TCP, read with `cargo vita logs`. This setup sends to the PC on port 9999, so `.envrc` sets `VITA_LOG_PORT=9999` (cargo-vita defaults to 8888).
- The downloads folder on this Vita is `ux0:downloads/`, while cargo-vita defaults to `ux0:/download/` and fails with `550`. Upload with `cargo vita upload -s <vpk> -d ux0:/downloads/`.
- `VITA_IP` goes in `.envrc` or the shell environment, never in a committed file (`.envrc` is gitignored).
