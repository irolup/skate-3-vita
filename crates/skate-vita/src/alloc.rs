//! A thread-caching allocator in front of newlib's `malloc`.
//!
//! newlib's `malloc` takes one lock for every call. With the retail UI running on the main thread (a
//! few thousand small allocations per frame) the simulation thread, whose walking ticks make about a
//! thousand of its own, waited on that lock for tens of milliseconds at a time: the walk scenario
//! went from 10.6 ms per tick (before the retail UI: no dropped ticks) to 13.4 ms with spikes of 90 ms, and back
//! to 10.8 ms with the UI off. Small blocks (up to 4 KiB) are now kept in per-thread free lists:
//! a thread allocates and frees them without any lock, and only a miss, or a list that is full,
//! reaches `malloc`. Larger blocks go straight to it.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Bytes of live blocks larger than a size class (almost all of the heap), and their peak: the
/// `HEAP` log line. newlib's `mallinfo` walks the whole heap under the malloc lock: 265 ms on a
/// 150 MB heap, every ten seconds, with the simulation waiting.
static LARGE_LIVE: AtomicUsize = AtomicUsize::new(0);
static LARGE_PEAK: AtomicUsize = AtomicUsize::new(0);

/// Bytes of small blocks newlib handed out (live blocks and the ones in the per-thread free
/// lists): with [`large_blocks`] the heap the program holds (the peds' many small blocks were not
/// in the `HEAP` line, which read 146 MB when the 304 MB heap ran out).
static SMALL_HELD: AtomicUsize = AtomicUsize::new(0);

pub fn small_blocks() -> usize {
    SMALL_HELD.load(Ordering::Relaxed)
}

/// (live, peak) bytes in blocks larger than 4 KiB.
pub fn large_blocks() -> (usize, usize) {
    (LARGE_LIVE.load(Ordering::Relaxed), LARGE_PEAK.load(Ordering::Relaxed))
}

/// The main thread's buffer for what passes through on its way to the GPU: the pause menu's capture (960 x 544, RGBA)
/// and the Challenge Map's images (1024 x 1024). Reserved at boot, it stays. Allocated each time, they took blocks from
/// the free top of the heap, which small blocks then filled: the heap's used part climbed 275 -> 301 MiB in five minutes
/// with the same 270 MiB in use, 29 MiB free in 15,000 pieces, until 4 MiB no longer fitted.
pub const SCRATCH_BYTES: usize = 4 << 20;
pub static SCRATCH: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

/// The biggest block asked for since the last [`take_biggest_asked`]: what a load needs in one unbroken piece.
static BIGGEST_ASKED: AtomicUsize = AtomicUsize::new(0);

pub fn take_biggest_asked() -> usize {
    BIGGEST_ASKED.swap(0, Ordering::Relaxed)
}

/// The largest block `malloc` can give right now, to the MiB below (a few trial allocations): how much of the free heap
/// is still in one piece. For the `LOAD` lines only: every trial takes the malloc lock, so not for use per frame.
pub fn largest_free_mib() -> usize {
    let (mut fits, mut too_big) = (0, 304);
    while too_big - fits > 1 {
        let size = (fits + too_big) / 2;
        let layout = unsafe { Layout::from_size_align_unchecked(size << 20, 8) };
        // Opaque: the optimiser removes a malloc and free that nothing uses and takes the malloc as successful.
        let p = std::hint::black_box(unsafe { System.alloc(layout) });
        if p.is_null() {
            too_big = size;
        } else {
            unsafe { System.dealloc(p, layout) };
            fits = size;
        }
    }
    fits
}

fn grew(by: usize) {
    BIGGEST_ASKED.fetch_max(by, Ordering::Relaxed);
    let now = LARGE_LIVE.fetch_add(by, Ordering::Relaxed) + by;
    LARGE_PEAK.fetch_max(now, Ordering::Relaxed);
}

/// Memory accounting (`mem-log`): who holds the heap, by kind of thread. A large block carries the
/// tag of the thread that allocated it in a 16-byte header (exact); small blocks are counted by
/// the thread that allocates or frees them (a block freed by another thread moves its bytes
/// there: the sum is right, the split approximate).
#[cfg(feature = "mem-log")]
pub mod tags {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicIsize, Ordering};

    pub const NAMES: [&str; 7] = ["main", "sim", "window", "tile", "audio", "load", "other"];
    const OTHER: u8 = 6;
    pub const HEADER: usize = 16;
    pub static LARGE: [AtomicIsize; 7] = [const { AtomicIsize::new(0) }; 7];
    pub static SMALL: [AtomicIsize; 7] = [const { AtomicIsize::new(0) }; 7];

    // Set by the thread itself (`tag_thread`): asking std for the thread's name from inside the
    // allocator is not safe (a thread that is ending has no name any more, and std panics).
    std::thread_local! { static TAG: Cell<u8> = const { Cell::new(OTHER) }; }

    pub fn set(kind: &str) {
        let tag = NAMES.iter().position(|n| *n == kind).map_or(OTHER, |i| i as u8);
        let _ = TAG.try_with(|t| t.set(tag));
    }

    /// The current thread's tag.
    pub fn current() -> u8 {
        TAG.try_with(Cell::get).unwrap_or(OTHER)
    }

    pub fn small(by: isize) {
        SMALL[current() as usize].fetch_add(by, Ordering::Relaxed);
    }

    /// `name=MB` per tag of one table.
    pub fn line(table: &[AtomicIsize; 7]) -> String {
        NAMES.iter().zip(table).map(|(n, b)| format!("{n}={:.1}", b.load(Ordering::Relaxed) as f64 / 1e6)).collect::<Vec<_>>().join(" ")
    }
}

/// Tags the current thread: for the `mem-log` breakdown (one of `tags::NAMES`) and for where its large blocks go (see
/// [`game_area`]): the threads whose blocks go with the world ("sim", "window", "tile") stay out of the game's area.
pub fn tag_thread(kind: &str) {
    let _ = WORLD_THREAD.try_with(|w| w.set(matches!(kind, "sim" | "window" | "tile")));
    #[cfg(feature = "mem-log")]
    tags::set(kind);
}

thread_local! {
    static WORLD_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// Blocks up to this size go to the game's area, from the threads not working for the world. Larger ones are few: the
/// audio's 39.7 MB, allocated once.
const GAME_AREA_BLOCK: usize = 2 << 20;

/// A part of the heap with its own allocator: an `mspace` of the system's library (what vitaGL's pools use), made on
/// first use. Off the Vita there is no such thing: no block ever comes from one.
mod areas {
    use std::alloc::Layout;
    #[cfg(target_os = "vita")]
    use std::alloc::{GlobalAlloc, System};
    #[cfg(target_os = "vita")]
    use std::ffi::c_void;
    #[cfg(target_os = "vita")]
    use std::sync::atomic::{AtomicBool, AtomicPtr};
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub struct Area {
        size: usize,
        base: AtomicUsize,
        used: AtomicUsize,
        #[cfg(target_os = "vita")]
        space: AtomicPtr<c_void>,
        #[cfg(target_os = "vita")]
        lock: AtomicBool,
    }

    #[cfg(target_os = "vita")]
    unsafe extern "C" {
        fn sceClibMspaceCreate(memblock: *mut c_void, size: u32) -> *mut c_void;
        fn sceClibMspaceMemalign(mspace: *mut c_void, alignment: u32, size: u32) -> *mut c_void;
        fn sceClibMspaceFree(mspace: *mut c_void, block: *mut c_void);
        fn sceClibMspaceMallocUsableSize(block: *mut c_void) -> u32;
    }

    impl Area {
        pub const fn new(size: usize) -> Self {
            Self {
                size,
                base: AtomicUsize::new(0),
                used: AtomicUsize::new(0),
                #[cfg(target_os = "vita")]
                space: AtomicPtr::new(std::ptr::null_mut()),
                #[cfg(target_os = "vita")]
                lock: AtomicBool::new(false),
            }
        }

        #[cfg(target_os = "vita")]
        fn lock(&self) {
            let mut spins = 0;
            while self.lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
                spins += 1;
                if spins > 64 {
                    std::thread::sleep(std::time::Duration::from_micros(1));
                } else {
                    std::hint::spin_loop();
                }
            }
        }

        #[cfg(target_os = "vita")]
        fn unlock(&self) {
            self.lock.store(false, Ordering::Release);
        }

        /// Bytes of live blocks.
        pub fn used(&self) -> usize {
            self.used.load(Ordering::Relaxed)
        }

        #[inline]
        pub fn contains(&self, p: *mut u8) -> bool {
            let base = self.base.load(Ordering::Acquire);
            base != 0 && (p as usize).wrapping_sub(base) < self.size
        }

        /// A block, or null: no room.
        #[cfg(target_os = "vita")]
        pub fn alloc(&self, layout: Layout) -> *mut u8 {
            self.lock();
            let mut space = self.space.load(Ordering::Relaxed);
            if space.is_null() && self.base.load(Ordering::Relaxed) == 0 {
                // SAFETY: a valid non-zero layout; never freed. A failure leaves the base at 1: not tried again.
                let memory = unsafe { System.alloc(Layout::from_size_align_unchecked(self.size, 64 << 10)) };
                if memory.is_null() {
                    self.base.store(1, Ordering::Release);
                } else {
                    // SAFETY: the block is ours, for good.
                    space = unsafe { sceClibMspaceCreate(memory.cast(), self.size as u32) };
                    self.space.store(space, Ordering::Relaxed);
                    self.base.store(memory as usize, Ordering::Release);
                }
            }
            let p = if space.is_null() {
                std::ptr::null_mut()
            } else {
                // SAFETY: under the lock, as every use of the space.
                unsafe { sceClibMspaceMemalign(space, layout.align().max(16) as u32, layout.size() as u32).cast::<u8>() }
            };
            if !p.is_null() {
                // What it can hold, as `free_counted` takes off: `free` takes the same for a fixed-size caller's block.
                self.used.fetch_add(self.usable(p), Ordering::Relaxed);
            }
            self.unlock();
            p
        }

        #[cfg(not(target_os = "vita"))]
        pub fn alloc(&self, _layout: Layout) -> *mut u8 {
            std::ptr::null_mut()
        }

        /// The usable size of a live block of the area (the C code's calls do not say).
        #[cfg(target_os = "vita")]
        pub fn usable(&self, p: *mut u8) -> usize {
            // SAFETY: a live block of a space.
            unsafe { sceClibMspaceMallocUsableSize(p.cast()) as usize }
        }

        /// # Safety
        /// As [`Self::free`], whatever size the block was asked for with: it counts what the block can hold.
        #[cfg(target_os = "vita")]
        pub unsafe fn free_counted(&self, p: *mut u8) {
            let size = self.usable(p);
            self.lock();
            // SAFETY: under the lock; the block is the space's.
            unsafe { sceClibMspaceFree(self.space.load(Ordering::Relaxed), p.cast()) };
            self.used.fetch_sub(size, Ordering::Relaxed);
            self.unlock();
        }

        /// # Safety
        /// `p` came from [`Self::alloc`] with this `size` and is not in use.
        #[cfg(target_os = "vita")]
        pub unsafe fn free(&self, p: *mut u8, size: usize) {
            self.lock();
            // SAFETY: under the lock; the block is the space's.
            let _ = size;
            let size = self.usable(p);
            unsafe { sceClibMspaceFree(self.space.load(Ordering::Relaxed), p.cast()) };
            self.used.fetch_sub(size, Ordering::Relaxed);
            self.unlock();
        }

        /// # Safety
        /// Never called off the Vita: no block is ever in an area there.
        #[cfg(not(target_os = "vita"))]
        pub unsafe fn free(&self, _p: *mut u8, _size: usize) {}
    }
}

/// The large blocks that outlive a world: audio banks, menus, the main thread's. Allocated among the first world's
/// blocks they stayed when it left, and the next world found its free space cut up: 40 MB free in 26 stretches, 14 MB at
/// most, and 5 MB and 11 MB blocks refused on the second world change (the heap map, Industrial back to DownTown).
/// Here they have their own area, and what a world leaves behind joins up. 37.2 MB at most in play, around 30 MB usually.
/// What does not fit goes to `malloc` as before.
static GAME_AREA: areas::Area = areas::Area::new(36 << 20);

/// The collision windows' three buffer sets (30.4 MB) in a district, and in a park, which has no windows, the room for
/// its largest blocks: empty there, it is one piece whatever the heap around it looks like. MegaPark's collision asks
/// for 21.24 MB at once; after DownTown 86 MB of the heap were free and 20.4 MB the longest stretch. Reserved first at
/// boot, and only these use it, so nothing can settle in it.
///
/// 14 MiB more for a district's prop collision (84,000 triangles, 13.3 MB), reserved at load: growing, it doubled to 11
/// MB in play and was refused 6 minutes into DownTown, and idle until then this room cost the main heap too much. Also
/// what MegaPark's 13.69, 9.37 and 21.24 MB need side by side, with the smaller steps of its growth kept out.
static WINDOW_AREA: areas::Area = areas::Area::new(47 << 20);
/// A block at least this large looks for room in the window area: a park's largest (the pack it reads in one piece, 13.7
/// MB for MegaPark and freed before its collision's 21.2 MB), and in a district that area is full and it goes on to
/// `malloc`. Not lower: the game's 10.78 MB block lasts, and would keep the window buffers out for good.
const WINDOW_AREA_BLOCK: usize = 12 << 20;

thread_local! {
    static TO_WINDOW_AREA: Cell<bool> = const { Cell::new(false) };
}

/// A park after the first world is loaded (windows off, the window area empty): a large block that grows (a park's
/// collision: 1, 2, 4, 8.6 then 11.6 MB after DownTownSkatePark, 21.2 MB in MegaPark, each step asking for a new block
/// while the last is copied) moves to the window area when past 8 MiB. Its small steps stay out, or they take the room
/// the last one needs (21.24 MB refused). Not in the first world: what lasts the whole game is made then
/// and would stay there.
static PARK_GROWTH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const PARK_GROWTH_BLOCK: usize = 8 << 20;
/// In such a park its large blocks go there too: after 9 minutes in DownTown the main heap had 8 MiB in one piece, for
/// MegaPark's 9.4 MB. Its 13.69, 9.37 and 21.24 MB fit the area side by side (the first is freed before the last).
const PARK_BLOCK: usize = 4 << 20;
/// A world's block growing past this looks for room in the window area, also in a district. Not lower: the 5.5 MB it
/// grows from must not take the room the 11 MB needs while it is copied.
const DISTRICT_GROWTH_BLOCK: usize = 8 << 20;

pub fn set_park_growth(on: bool) {
    PARK_GROWTH.store(on, Ordering::Relaxed);
}

pub fn park_growth() -> bool {
    PARK_GROWTH.load(Ordering::Relaxed)
}

/// Runs `reserve`, whose blocks go to the window area: the collision windows' buffer sets.
pub fn in_window_area<T>(reserve: impl FnOnce() -> T) -> T {
    let _ = TO_WINDOW_AREA.try_with(|f| f.set(true));
    let result = reserve();
    let _ = TO_WINDOW_AREA.try_with(|f| f.set(false));
    result
}

/// Bytes of live blocks in the game's area and in the window area.
pub fn game_area_used() -> usize {
    GAME_AREA.used()
}

pub fn window_area_used() -> usize {
    WINDOW_AREA.used()
}

#[inline]
unsafe fn raw_alloc(layout: Layout) -> *mut u8 {
    // An over-aligned small block (std's channels: 32-byte alignment): `memalign` would put it in the main heap with a
    // sliver beside it. Its free needs nothing here: `System`'s `free` is wrapped and knows the area.
    if layout.size() <= LARGE {
        let p = C_AREA.alloc(layout);
        if !p.is_null() {
            return p;
        }
    }
    let parks_block = layout.size() >= PARK_BLOCK && PARK_GROWTH.load(Ordering::Relaxed);
    if parks_block || layout.size() >= WINDOW_AREA_BLOCK || TO_WINDOW_AREA.try_with(Cell::get).unwrap_or(false) {
        let p = WINDOW_AREA.alloc(layout);
        if !p.is_null() {
            return p;
        }
    }
    if layout.size() <= GAME_AREA_BLOCK && !WORLD_THREAD.try_with(Cell::get).unwrap_or(false) {
        let p = GAME_AREA.alloc(layout);
        if !p.is_null() {
            return p;
        }
    }
    unsafe { System.alloc(layout) }
}

#[inline]
unsafe fn raw_dealloc(p: *mut u8, layout: Layout) {
    if GAME_AREA.contains(p) {
        unsafe { GAME_AREA.free(p, layout.size()) }
    } else if WINDOW_AREA.contains(p) {
        unsafe { WINDOW_AREA.free(p, layout.size()) }
    } else if C_AREA.contains(p) {
        unsafe { C_AREA.free(p, layout.size()) }
    } else {
        unsafe { System.dealloc(p, layout) }
    }
}

unsafe fn raw_realloc(p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    // A block growing into the window area's sizes moves there (MegaPark's collision grows 2.16 -> 21.24 MB).
    let grows = new_size > layout.size();
    let grows_in_park = grows && new_size >= PARK_GROWTH_BLOCK && PARK_GROWTH.load(Ordering::Relaxed);
    let grows_in_district = grows && new_size >= DISTRICT_GROWTH_BLOCK && WORLD_THREAD.try_with(Cell::get).unwrap_or(false);
    let grows_in_park = grows_in_park || grows_in_district;
    if !GAME_AREA.contains(p) && !WINDOW_AREA.contains(p) && !C_AREA.contains(p) && new_size > LARGE && new_size < WINDOW_AREA_BLOCK && !grows_in_park {
        return unsafe { System.realloc(p, layout, new_size) };
    }
    let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
    let mut q = if grows_in_park { WINDOW_AREA.alloc(new_layout) } else { ptr::null_mut() };
    if q.is_null() {
        q = unsafe { raw_alloc(new_layout) };
    }
    if !q.is_null() {
        unsafe {
            ptr::copy_nonoverlapping(p, q, layout.size().min(new_size));
            raw_dealloc(p, layout);
        }
    }
    q
}

/// A `MEMSTAGE` log line (`mem-log`): the heap by thread kind at a point of the loading.
pub fn stage(label: &str) {
    #[cfg(feature = "mem-log")]
    println!(
        "MEMSTAGE {label}: large {:.1} MB [{}], small held {:.1} MB, live [{}]",
        LARGE_LIVE.load(Ordering::Relaxed) as f64 / 1e6,
        tags::line(&tags::LARGE),
        SMALL_HELD.load(Ordering::Relaxed) as f64 / 1e6,
        tags::line(&tags::SMALL)
    );
    #[cfg(not(feature = "mem-log"))]
    let _ = label;
}

/// `HEAPMAP` log lines (`mem-log`): newlib's heap from its start, each free stretch of 256 KiB and more with what lies
/// between them, by owner: what keeps the free space from being one piece. Walks the chunks under the malloc lock (a
/// few ms), printing after it, because printing can allocate.
#[cfg(all(feature = "mem-log", target_os = "vita"))]
pub fn heap_map(label: &str) {
    use std::ffi::c_void;
    unsafe extern "C" {
        fn __malloc_lock(reent: *mut c_void);
        fn __malloc_unlock(reent: *mut c_void);
        fn __getreent() -> *mut c_void;
        static __malloc_sbrk_base: *mut u8;
    }
    #[derive(Clone, Copy, Default)]
    struct Row {
        /// Bytes per owner in blocks of 4 KiB and more, then in smaller blocks, with their count; small free gaps;
        /// the largest block and its owner; the free stretch after it all.
        large: [usize; 7],
        small: usize,
        small_blocks: u32,
        gaps: usize,
        biggest: usize,
        biggest_tag: u8,
        free: usize,
    }
    const ROWS: usize = 96;
    let mut rows = [Row::default(); ROWS];
    // Small blocks still in the main heap, by size: 8-byte steps up to 256 bytes, the last for all above.
    let mut tiny = [0u32; 33];
    let (mut used_rows, mut stopped) = (0, false);
    let (_, arena, _, _) = vita_platform::vita::heap_detail();
    unsafe {
        let reent = __getreent();
        __malloc_lock(reent);
        let base = __malloc_sbrk_base as usize;
        let end = base + arena;
        // The first chunk: its user pointer is 8-aligned.
        let mut p = if (base + 8) % 8 == 0 { base } else { base + 8 - (base + 8) % 8 };
        let mut row = Row::default();
        while p + 8 < end {
            let size = *((p + 4) as *const usize) & !3;
            if size < 16 || size % 8 != 0 || p + size > end {
                stopped = true;
                break;
            }
            let next = p + size;
            // The last chunk is the top, always free; otherwise the next chunk's flag tells.
            let in_use = next + 8 <= end && next + 8 < end && *((next + 4) as *const usize) & 1 != 0;
            if in_use {
                if size >= LARGE {
                    let tag = *((p + 8) as *const u8);
                    let owner = if (tag as usize) < 7 { tag } else { 6 };
                    row.large[owner as usize] += size;
                    if size > row.biggest {
                        (row.biggest, row.biggest_tag) = (size, owner);
                    }
                } else {
                    row.small += size;
                    row.small_blocks += 1;
                    tiny[(size / 8).min(32)] += 1;
                }
            } else if size >= 256 << 10 {
                row.free = size;
                if used_rows < ROWS {
                    rows[used_rows] = row;
                    used_rows += 1;
                }
                row = Row::default();
            } else {
                row.gaps += size;
            }
            p = next;
        }
        if used_rows < ROWS {
            rows[used_rows] = row;
            used_rows += 1;
        }
        __malloc_unlock(reent);
    }
    let mb = |b: usize| b as f64 / 1e6;
    println!("HEAPMAP {label}: arena {:.1} MB, {used_rows} stretches{}; game area {:.1} MB used", mb(arena), if stopped { ", WALK STOPPED" } else { "" }, mb(GAME_AREA.used()));
    for r in &rows[..used_rows] {
        let owners: Vec<String> = tags::NAMES.iter().zip(r.large).filter(|(_, b)| *b > 0).map(|(n, b)| format!("{n}={:.2}", mb(b))).collect();
        println!(
            "HEAPMAP  used [{}] small {:.2} in {}, gaps {:.2}; biggest {:.2} {}; then FREE {:.2}",
            owners.join(" "),
            mb(r.small),
            r.small_blocks,
            mb(r.gaps),
            mb(r.biggest),
            tags::NAMES[r.biggest_tag as usize],
            mb(r.free)
        );
    }
    let sizes: Vec<String> = tiny.iter().enumerate().filter(|(_, n)| **n > 0).map(|(i, n)| if i == 32 { format!(">=256:{n}") } else { format!("{}:{n}", i * 8) }).collect();
    println!("HEAPMAP tiny blocks by size: {}; C area {:.2} MB, {} blocks spilled", sizes.join(" "), mb(C_AREA.used()), C_SPILLED.load(Ordering::Relaxed));
    callers::print(c_malloc::anchor());
}

#[cfg(not(all(feature = "mem-log", target_os = "vita")))]
pub fn heap_map(_label: &str) {}

/// A large block of this layout carries the tag header (`mem-log`).
#[cfg(feature = "mem-log")]
#[inline]
fn tagged(layout: &Layout) -> bool {
    layout.align() <= tags::HEADER
}

/// A `BIG` log line (`mem-log`): who holds the blocks of a megabyte and more. They come by the dozen, not per frame.
#[cfg(feature = "mem-log")]
fn big(what: &str, size: usize) {
    if size >= 1 << 20 {
        println!("BIG {what} {:.2} MB on {}", size as f64 / 1e6, tags::NAMES[tags::current() as usize]);
    }
}

#[cfg(feature = "mem-log")]
unsafe fn large_alloc(layout: Layout) -> *mut u8 {
    big("alloc", layout.size());
    if !tagged(&layout) {
        return unsafe { raw_alloc(layout) };
    }
    let base = unsafe { raw_alloc(Layout::from_size_align_unchecked(layout.size() + tags::HEADER, layout.align())) };
    if base.is_null() {
        return base;
    }
    let tag = tags::current();
    unsafe { *base = tag };
    tags::LARGE[tag as usize].fetch_add(layout.size() as isize, Ordering::Relaxed);
    unsafe { base.add(tags::HEADER) }
}

#[cfg(feature = "mem-log")]
unsafe fn large_dealloc(p: *mut u8, layout: Layout) {
    big("free", layout.size());
    if !tagged(&layout) {
        return unsafe { raw_dealloc(p, layout) };
    }
    let base = unsafe { p.sub(tags::HEADER) };
    tags::LARGE[unsafe { *base } as usize].fetch_sub(layout.size() as isize, Ordering::Relaxed);
    unsafe { raw_dealloc(base, Layout::from_size_align_unchecked(layout.size() + tags::HEADER, layout.align())) }
}

#[cfg(feature = "mem-log")]
unsafe fn large_realloc(p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    big("realloc from", layout.size());
    big("realloc to", new_size);
    if !tagged(&layout) {
        return unsafe { raw_realloc(p, layout, new_size) };
    }
    let base = unsafe { p.sub(tags::HEADER) };
    let tag = unsafe { *base } as usize;
    let q = unsafe { raw_realloc(base, Layout::from_size_align_unchecked(layout.size() + tags::HEADER, layout.align()), new_size + tags::HEADER) };
    if q.is_null() {
        return q;
    }
    tags::LARGE[tag].fetch_add(new_size as isize - layout.size() as isize, Ordering::Relaxed);
    unsafe { q.add(tags::HEADER) }
}

#[cfg(not(feature = "mem-log"))]
#[inline]
unsafe fn large_alloc(layout: Layout) -> *mut u8 {
    unsafe { raw_alloc(layout) }
}

#[cfg(not(feature = "mem-log"))]
#[inline]
unsafe fn large_dealloc(p: *mut u8, layout: Layout) {
    unsafe { raw_dealloc(p, layout) }
}

#[cfg(not(feature = "mem-log"))]
#[inline]
unsafe fn large_realloc(p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    unsafe { raw_realloc(p, layout, new_size) }
}

/// A large allocation failed: which thread asked and for how much (the abort message that
/// follows has neither). Small allocations still work at that point, so printing is safe.
#[cold]
fn failed(size: usize) {
    BIGGEST_ASKED.fetch_max(size, Ordering::Relaxed);
    // Not only the first: a caller that can go without (the pause menu's capture) survives a failure.
    static LINES: AtomicUsize = AtomicUsize::new(0);
    if LINES.fetch_add(1, Ordering::Relaxed) < 8 {
        let thread = std::thread::current();
        println!(
            "ALLOC FAILED {size} bytes on thread {:?}; large blocks live {} peak {}",
            thread.name().unwrap_or("?"),
            LARGE_LIVE.load(Ordering::Relaxed),
            LARGE_PEAK.load(Ordering::Relaxed)
        );
    }
}

/// Size classes: 16-byte steps up to 512, then 64-byte steps up to 4096.
const SMALL: usize = 512;
const LARGE: usize = 4096;
const CLASSES: usize = SMALL / 16 + (LARGE - SMALL) / 64;
/// The most blocks a thread keeps per class (a thread that stops leaves them cached), and at most this many bytes of
/// them: 128 blocks of every class let one thread cache up to 35 MB. Blocks one thread allocates and another frees
/// (the tile loader's, freed by the main thread) filled the main thread's lists of the big classes, and DownTown's
/// small blocks grew 86 -> 124 MB beside 159 MB of large ones until a 194 KB block no longer fitted the 304 MB heap
/// (the streaming benchmark at 24 m/s). Classes up to 512 bytes keep their 128.
const KEEP: u32 = 128;
const KEEP_BYTES: usize = 64 << 10;

#[inline]
fn keep(slot: usize) -> u32 {
    ((KEEP_BYTES / slot_size(slot)) as u32).clamp(16, KEEP)
}

#[inline]
fn class_of(size: usize) -> usize {
    if size <= SMALL { size.max(1).div_ceil(16) - 1 } else { SMALL / 16 + (size - SMALL).div_ceil(64) - 1 }
}

#[inline]
fn class_size(class: usize) -> usize {
    if class < SMALL / 16 { (class + 1) * 16 } else { SMALL + (class - SMALL / 16 + 1) * 64 }
}

/// Two banks of classes, by alignment. A block for 8-byte alignment or less comes
/// from `malloc`; one for 16 from `memalign` (Rust's `System` above 8 on the Vita). All blocks used
/// to be 16-aligned, and newlib's `memalign` leaves a 16-32 byte free sliver beside every block it
/// returns: DownTown's heap had 49 MiB free in 844,000 pieces (one per small block), which no
/// larger request could use, while the heap grew to its limit.
const SLOTS: usize = 2 * CLASSES;

#[inline]
fn slot_of(layout: &Layout) -> usize {
    class_of(layout.size()) + if layout.align() > 8 { CLASSES } else { 0 }
}

#[inline]
fn slot_size(slot: usize) -> usize {
    class_size(slot % CLASSES)
}

/// What a slot's blocks are asked of `System` with.
#[inline]
fn slot_layout(slot: usize) -> Layout {
    unsafe { Layout::from_size_align_unchecked(slot_size(slot), if slot < CLASSES { 8 } else { 16 }) }
}

/// The small blocks' own region. Among the large blocks they filled every gap a large block left, and the heap ran out
/// of unbroken space with room to spare: DownTown after six minutes, 270 MiB in use and 23 MiB free in 20,000 pieces of
/// 1.2 KiB on average, until 955 KB no longer fitted. Here they share pages of one size class; an empty page goes back
/// to any class. Worlds differ too much for one size: DownTown needs 118 MiB of pages and MegaPark's large blocks need
/// that memory, so beyond a fixed [`REGION`] the pages come in chunks of 1 MiB ([`CHUNK`]), given back when empty. At a
/// fixed 112 MiB DownTown's overflow into `malloc` left MegaPark's 21 MB block 30 MB in stretches of at most 3 MB.
mod slab {
    use super::{class_size, CLASSES};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::UnsafeCell;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// What every world uses: MegaPark's 74 MiB of small blocks and a page slack, and a part of DownTown's 118 that
    /// would otherwise be chunks in the main heap. At 118 the main heap was too small for DownTown itself (3 MB refused
    /// right after its load, 177 MB of large blocks live).
    pub const REGION: usize = 92 << 20;
    const CHUNK: usize = 1 << 20;
    /// Chunks at most (DownTown wants 34): past them small blocks go to `malloc` as a last resort.
    const CHUNKS: usize = 56;
    const PAGE: usize = 64 << 10;
    const PAGES_PER_CHUNK: usize = CHUNK / PAGE;
    const FIXED_PAGES: usize = REGION / PAGE;
    const PAGES: usize = FIXED_PAGES + CHUNKS * PAGES_PER_CHUNK;

    /// Links are page numbers + 1, block offsets in a page + 1 (0: none).
    #[derive(Clone, Copy)]
    struct Page {
        next: u16,
        prev: u16,
        class: u16,
        live: u16,
        carved: u16,
        /// `mem-log`: the kind of thread that last took a block of this page.
        #[allow(dead_code)]
        tag: u8,
        free: u32,
    }

    struct State {
        pages: [Page; PAGES],
        /// Pages per class with room: holds a page exactly while it is neither full nor empty. The fixed region's, and
        /// the chunks' apart: a block goes to a chunk only when nothing else has room, or the chunks stay in use for good.
        partial: [u16; CLASSES],
        chunk_partial: [u16; CLASSES],
        free_pages: u16,
        bumped: u16,
        no_region: bool,
        /// Free pages per chunk in use, a bit each (all set: the chunk goes back).
        chunk_free: [u16; CHUNKS],
    }

    struct Shared(UnsafeCell<State>);
    // SAFETY: only used while holding `LOCK`.
    unsafe impl Sync for Shared {}

    static STATE: Shared = Shared(UnsafeCell::new(State {
        pages: [Page { next: 0, prev: 0, class: 0, live: 0, carved: 0, tag: 0, free: 0 }; PAGES],
        partial: [0; CLASSES],
        chunk_partial: [0; CLASSES],
        free_pages: 0,
        bumped: 0,
        no_region: false,
        chunk_free: [0; CHUNKS],
    }));
    static BASE: AtomicUsize = AtomicUsize::new(0);
    /// The chunks' addresses (0: none). Read without the lock by `contains`: a block's chunk cannot go while it lives.
    static CHUNK_AT: [AtomicUsize; CHUNKS] = [const { AtomicUsize::new(0) }; CHUNKS];
    static PAGES_USED: AtomicUsize = AtomicUsize::new(0);
    static CHUNK_GROWTH: AtomicUsize = AtomicUsize::new(0);
    static LOCK: AtomicBool = AtomicBool::new(false);

    /// Held for a few list operations. A sleep after a short spin: a thread pinned to the core of a lower-priority
    /// holder would otherwise spin forever. Not a `Mutex`: std's may allocate.
    fn lock() {
        let mut spins = 0;
        while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            spins += 1;
            if spins > 64 {
                std::thread::sleep(std::time::Duration::from_micros(1));
            } else {
                std::hint::spin_loop();
            }
        }
    }

    fn unlock() {
        LOCK.store(false, Ordering::Release);
    }

    /// Bytes of the region in pages holding blocks.
    pub fn bytes_used() -> usize {
        PAGES_USED.load(Ordering::Relaxed) * PAGE
    }

    /// The chunk holding `p`.
    fn chunk_of(p: usize) -> Option<usize> {
        CHUNK_AT.iter().position(|c| {
            let at = c.load(Ordering::Acquire);
            at != 0 && p.wrapping_sub(at) < CHUNK
        })
    }

    #[inline]
    pub fn contains(p: *mut u8) -> bool {
        let base = BASE.load(Ordering::Acquire);
        (base != 0 && (p as usize).wrapping_sub(base) < REGION) || chunk_of(p as usize).is_some()
    }

    #[cfg(test)]
    pub fn in_fixed(p: *mut u8) -> bool {
        (p as usize).wrapping_sub(BASE.load(Ordering::Relaxed)) < REGION
    }

    /// A page's address (`index`: its number).
    fn page_start(index: usize) -> usize {
        if index < FIXED_PAGES {
            BASE.load(Ordering::Relaxed) + index * PAGE
        } else {
            let (chunk, page) = ((index - FIXED_PAGES) / PAGES_PER_CHUNK, (index - FIXED_PAGES) % PAGES_PER_CHUNK);
            CHUNK_AT[chunk].load(Ordering::Relaxed) + page * PAGE
        }
    }

    /// A free page of the chunks, or of a new one. The number, or none: all chunks full, or no block of the size.
    fn chunk_page(s: &mut State) -> Option<usize> {
        let found = (0..CHUNKS).find(|&c| s.chunk_free[c] != 0).or_else(|| {
            let slot = CHUNK_AT.iter().position(|c| c.load(Ordering::Relaxed) == 0)?;
            // SAFETY: a valid non-zero layout; freed in `free` with the same one.
            let memory = unsafe { System.alloc(Layout::from_size_align_unchecked(CHUNK, PAGE)) };
            if memory.is_null() {
                return None;
            }
            CHUNK_AT[slot].store(memory as usize, Ordering::Release);
            s.chunk_free[slot] = u16::MAX;
            CHUNK_GROWTH.fetch_add(1, Ordering::Relaxed);
            Some(slot)
        })?;
        let page = s.chunk_free[found].trailing_zeros() as usize;
        s.chunk_free[found] &= !(1 << page);
        Some(FIXED_PAGES + found * PAGES_PER_CHUNK + page)
    }

    /// The list a page's class keeps it on while it has room.
    fn list(s: &mut State, class: usize, id: u16) -> &mut u16 {
        if id as usize - 1 < FIXED_PAGES { &mut s.partial[class] } else { &mut s.chunk_partial[class] }
    }

    fn unlink(s: &mut State, class: usize, id: u16) {
        let Page { next, prev, .. } = s.pages[id as usize - 1];
        if prev != 0 {
            s.pages[prev as usize - 1].next = next;
        } else {
            *list(s, class, id) = next;
        }
        if next != 0 {
            s.pages[next as usize - 1].prev = prev;
        }
    }

    fn link(s: &mut State, class: usize, id: u16) {
        let head = *list(s, class, id);
        s.pages[id as usize - 1].next = head;
        s.pages[id as usize - 1].prev = 0;
        if head != 0 {
            s.pages[head as usize - 1].prev = id;
        }
        *list(s, class, id) = id;
    }

    /// A page with room for `class` when the fixed region's have none: a new one of the region, or with `chunks` a chunk's
    /// that has room, or a new one of a chunk; on the class's list. 0: none.
    fn take_page(s: &mut State, class: usize, chunks: bool) -> u16 {
        if BASE.load(Ordering::Relaxed) == 0 {
            if s.no_region {
                return 0;
            }
            // SAFETY: a valid non-zero layout; never freed.
            let region = unsafe { System.alloc(Layout::from_size_align_unchecked(REGION, PAGE)) };
            if region.is_null() {
                s.no_region = true;
                return 0;
            }
            BASE.store(region as usize, Ordering::Release);
        }
        let id = if s.free_pages != 0 {
            let id = s.free_pages;
            s.free_pages = s.pages[id as usize - 1].next;
            id
        } else if (s.bumped as usize) < FIXED_PAGES {
            s.bumped += 1;
            s.bumped
        } else if !chunks {
            return 0;
        } else if s.chunk_partial[class] != 0 {
            return s.chunk_partial[class];
        } else {
            match chunk_page(s) {
                Some(index) => index as u16 + 1,
                None => return 0,
            }
        };
        s.pages[id as usize - 1] = Page { next: 0, prev: 0, class: class as u16, live: 0, carved: 0, tag: 0, free: 0 };
        link(s, class, id);
        PAGES_USED.fetch_add(1, Ordering::Relaxed);
        id
    }

    /// A block of `class`, or null when the region is full (with `chunks` false: its fixed part). 16-aligned: pages start
    /// on a page boundary and every class size is a multiple of 16.
    pub fn alloc(class: usize, chunks: bool) -> *mut u8 {
        lock();
        // SAFETY: under the lock.
        let s = unsafe { &mut *STATE.0.get() };
        let mut id = s.partial[class];
        if id == 0 {
            id = take_page(s, class, chunks);
            if id == 0 {
                unlock();
                return ptr::null_mut();
            }
        }
        let size = class_size(class);
        let start = page_start(id as usize - 1);
        let page = &mut s.pages[id as usize - 1];
        let offset = if page.free != 0 {
            let offset = page.free as usize - 1;
            // SAFETY: a free block's first word is the next free offset + 1.
            page.free = unsafe { *((start + offset) as *const u32) };
            offset
        } else {
            page.carved += 1;
            (page.carved as usize - 1) * size
        };
        page.live += 1;
        #[cfg(feature = "mem-log")]
        {
            page.tag = super::tags::current();
        }
        if page.live as usize == PAGE / size {
            unlink(s, class, id);
        }
        unlock();
        (start + offset) as *mut u8
    }

    /// Gives back a block of this region.
    ///
    /// # Safety
    /// `p` came from [`alloc`] and is not in use.
    pub unsafe fn free(p: *mut u8) {
        let base = BASE.load(Ordering::Relaxed);
        let at = (p as usize).wrapping_sub(base);
        let (index, offset) = if base != 0 && at < REGION {
            (at / PAGE, at % PAGE)
        } else {
            let chunk = chunk_of(p as usize).expect("a block of the region");
            let at = p as usize - CHUNK_AT[chunk].load(Ordering::Relaxed);
            (FIXED_PAGES + chunk * PAGES_PER_CHUNK + at / PAGE, at % PAGE)
        };
        let id = index as u16 + 1;
        lock();
        // SAFETY: under the lock.
        let s = unsafe { &mut *STATE.0.get() };
        let class = s.pages[index].class as usize;
        if s.pages[index].live as usize == PAGE / class_size(class) {
            link(s, class, id);
        }
        let page = &mut s.pages[index];
        // SAFETY: the block is ours; its first word holds the list while it is free.
        unsafe { *(p as *mut u32) = page.free };
        page.free = offset as u32 + 1;
        page.live -= 1;
        let mut chunk_memory = 0;
        if page.live == 0 {
            unlink(s, class, id);
            PAGES_USED.fetch_sub(1, Ordering::Relaxed);
            if index < FIXED_PAGES {
                s.pages[index].next = s.free_pages;
                s.free_pages = id;
            } else {
                let (chunk, page) = ((index - FIXED_PAGES) / PAGES_PER_CHUNK, (index - FIXED_PAGES) % PAGES_PER_CHUNK);
                s.chunk_free[chunk] |= 1 << page;
                if s.chunk_free[chunk] == u16::MAX {
                    s.chunk_free[chunk] = 0;
                    chunk_memory = CHUNK_AT[chunk].swap(0, Ordering::AcqRel);
                }
            }
        }
        unlock();
        if chunk_memory != 0 {
            // SAFETY: from `chunk_page`, with this layout; every block of it is free and the chunk is off the table.
            unsafe { System.dealloc(chunk_memory as *mut u8, Layout::from_size_align_unchecked(CHUNK, PAGE)) };
        }
    }

    /// `mem-log`: what holds the chunks in use.
    #[cfg(feature = "mem-log")]
    pub struct Census {
        pub chunks: usize,
        pub pages: usize,
        pub live: usize,
        pub bytes: usize,
        /// Chunks by the blocks they hold: up to 4, 32, 256, more.
        pub by_live: [usize; 4],
        /// Live blocks per kind of thread that last took from their page.
        pub by_tag: [usize; 7],
        /// Live blocks per class.
        pub by_class: [u32; CLASSES],
    }

    #[cfg(feature = "mem-log")]
    pub fn census() -> Census {
        let mut c = Census { chunks: 0, pages: 0, live: 0, bytes: 0, by_live: [0; 4], by_tag: [0; 7], by_class: [0; CLASSES] };
        lock();
        // SAFETY: under the lock.
        let s = unsafe { &*STATE.0.get() };
        for chunk in 0..CHUNKS {
            if CHUNK_AT[chunk].load(Ordering::Relaxed) == 0 {
                continue;
            }
            c.chunks += 1;
            let mut held = 0;
            for page in 0..PAGES_PER_CHUNK {
                if s.chunk_free[chunk] & (1 << page) != 0 {
                    continue;
                }
                let p = &s.pages[FIXED_PAGES + chunk * PAGES_PER_CHUNK + page];
                c.pages += 1;
                held += p.live as usize;
                c.bytes += p.live as usize * class_size(p.class as usize);
                c.by_tag[(p.tag as usize).min(6)] += p.live as usize;
                c.by_class[p.class as usize] += p.live as u32;
            }
            c.live += held;
            c.by_live[match held {
                0..=4 => 0,
                5..=32 => 1,
                33..=256 => 2,
                _ => 3,
            }] += 1;
        }
        unlock();
        c
    }

    /// Chunks in use, and how many were ever taken.
    pub fn chunks() -> (usize, usize) {
        (CHUNK_AT.iter().filter(|c| c.load(Ordering::Relaxed) != 0).count(), CHUNK_GROWTH.load(Ordering::Relaxed))
    }
}

/// Small `malloc`s of the C code, and Rust's thread-local values and over-aligned small blocks, have an area of their own,
/// through the linker's `--wrap` (build.rs). In the main heap they were what kept cutting it: after a long ride 1,100 of
/// them, tens of KB in all, stood between the free stretches, 26 MB in 29 pieces with 4.2 MB at most.
///
/// Only the direct `malloc` and `calloc` come here. `_malloc_r` must stay the real one: newlib's `_memalign_r` and
/// `_realloc_r` call it and then treat the result as their own chunk, writing headers into it and freeing a pointer inside
/// it. Wrapped, that cut blocks of this area apart (a crash dump shows it: 0x11, 0x89 and 0x29 where one 0xC0 chunk was,
/// `_memalign_r(32, 0x80)`, and the mspace aborted on freeing the block before them).
///
/// What does not fit goes to the main heap as before. 0.1 MB in use a minute into DownTown.
static C_AREA: areas::Area = areas::Area::new(C_AREA_BYTES);
const C_AREA_BYTES: usize = 2 << 20;

/// Bytes of live blocks in the C code's area, and how many blocks found no room in it.
pub fn c_area_used() -> (usize, usize) {
    (C_AREA.used(), C_SPILLED.load(Ordering::Relaxed))
}

static C_SPILLED: AtomicUsize = AtomicUsize::new(0);

/// `mem-log`: who asks for small blocks, by return address and per kind of call.
#[cfg(all(feature = "mem-log", target_os = "vita"))]
mod callers {
    use super::LARGE;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const SLOTS: usize = 64;

    pub struct Table {
        at: [AtomicUsize; SLOTS],
        count: [AtomicUsize; SLOTS],
        bytes: [AtomicUsize; SLOTS],
    }

    pub static TO_AREA: Table = Table::new();
    pub static TO_HEAP: Table = Table::new();

    impl Table {
        const fn new() -> Self {
            Self { at: [const { AtomicUsize::new(0) }; SLOTS], count: [const { AtomicUsize::new(0) }; SLOTS], bytes: [const { AtomicUsize::new(0) }; SLOTS] }
        }

        /// Counts a request; lock-free, allocates nothing. Past the table's size a caller goes uncounted.
        #[inline]
        pub fn note(&self, at: usize, size: usize) {
            if size > LARGE {
                return;
            }
            for i in 0..SLOTS {
                let holds = self.at[i].load(Ordering::Relaxed);
                let mine = holds == at || (holds == 0 && self.at[i].compare_exchange(0, at, Ordering::Relaxed, Ordering::Relaxed).map_or_else(|now| now == at, |_| true));
                if mine {
                    self.count[i].fetch_add(1, Ordering::Relaxed);
                    self.bytes[i].fetch_add(size, Ordering::Relaxed);
                    return;
                }
            }
        }

        /// `address:count:bytes`, most called first.
        pub fn line(&self) -> String {
            let mut rows: Vec<(usize, usize, usize)> = (0..SLOTS)
                .map(|i| (self.at[i].load(Ordering::Relaxed), self.count[i].load(Ordering::Relaxed), self.bytes[i].load(Ordering::Relaxed)))
                .filter(|r| r.0 != 0)
                .collect();
            rows.sort_by_key(|r| std::cmp::Reverse(r.1));
            rows.iter().map(|(at, count, bytes)| format!("{at:x}:{count}:{bytes}")).collect::<Vec<_>>().join(" ")
        }
    }

    /// The caller of the `extern "C"` function this is the first thing in. A wrong read only misnames a caller.
    #[inline(always)]
    pub fn return_address() -> usize {
        let lr: usize;
        // SAFETY: reads a register.
        unsafe { core::arch::asm!("mov {}, lr", out(reg) lr, options(nomem, nostack, preserves_flags)) };
        lr
    }

    /// The `HEAPMAP` lines. The anchor's address maps these to the ELF's symbols.
    pub fn print(anchor: usize) {
        println!("HEAPMAP callers anchor __wrap_malloc={anchor:x}");
        println!("HEAPMAP callers to the area: {}", TO_AREA.line());
        println!("HEAPMAP callers staying in the heap: {}", TO_HEAP.line());
    }
}

#[cfg(target_os = "vita")]
mod c_malloc {
    use super::{C_AREA, C_SPILLED, LARGE};
    use std::alloc::Layout;
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::atomic::Ordering;

    unsafe extern "C" {
        fn __real_malloc(size: usize) -> *mut c_void;
        fn __real_free(p: *mut c_void);
        fn __real_realloc(p: *mut c_void, size: usize) -> *mut c_void;
        fn __real_calloc(count: usize, size: usize) -> *mut c_void;
        fn __real_malloc_usable_size(p: *mut c_void) -> usize;
        fn __real__free_r(reent: *mut c_void, p: *mut c_void);
        fn __real__realloc_r(reent: *mut c_void, p: *mut c_void, size: usize) -> *mut c_void;
        fn __real__malloc_usable_size_r(reent: *mut c_void, p: *mut c_void) -> usize;
    }

    /// A block of the area for `size` bytes, or null: too large, or no room.
    #[inline]
    fn small(size: usize) -> *mut c_void {
        if size > LARGE {
            return ptr::null_mut();
        }
        // SAFETY: a size of at most 4 KiB and alignment 8 make a valid layout.
        let p = C_AREA.alloc(unsafe { Layout::from_size_align_unchecked(size.max(1), 8) }).cast::<c_void>();
        if p.is_null() {
            C_SPILLED.fetch_add(1, Ordering::Relaxed);
        }
        p
    }

    /// `realloc` of a block of the area: the same block when it fits, else another one, of the area or of the main heap.
    /// A block of the main heap stays there: the real `realloc` handles it and only ever gives blocks of the main heap.
    unsafe fn moved(p: *mut c_void, size: usize) -> *mut c_void {
        let old = C_AREA.usable(p.cast());
        if size != 0 && size <= old {
            return p;
        }
        let q = if size == 0 { ptr::null_mut() } else { unsafe { __wrap_malloc(size) } };
        if !q.is_null() {
            unsafe { ptr::copy_nonoverlapping(p.cast::<u8>(), q.cast::<u8>(), old.min(size)) };
        }
        if !q.is_null() || size == 0 {
            unsafe { C_AREA.free_counted(p.cast()) };
        }
        q
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __wrap_malloc(size: usize) -> *mut c_void {
        #[cfg(feature = "mem-log")]
        super::callers::TO_AREA.note(super::callers::return_address(), size);
        let p = small(size);
        if p.is_null() { unsafe { __real_malloc(size) } } else { p }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __wrap_free(p: *mut c_void) {
        if C_AREA.contains(p.cast()) { unsafe { C_AREA.free_counted(p.cast()) } } else { unsafe { __real_free(p) } }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __wrap_realloc(p: *mut c_void, size: usize) -> *mut c_void {
        if p.is_null() {
            return unsafe { __wrap_malloc(size) };
        }
        if C_AREA.contains(p.cast()) {
            return unsafe { moved(p, size) };
        }
        unsafe { __real_realloc(p, size) }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __wrap_calloc(count: usize, size: usize) -> *mut c_void {
        #[cfg(feature = "mem-log")]
        super::callers::TO_AREA.note(super::callers::return_address(), count.saturating_mul(size));
        let bytes = count.saturating_mul(size);
        let p = small(bytes);
        if p.is_null() {
            return unsafe { __real_calloc(count, size) };
        }
        unsafe { ptr::write_bytes(p.cast::<u8>(), 0, bytes) };
        p
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __wrap_malloc_usable_size(p: *mut c_void) -> usize {
        if C_AREA.contains(p.cast()) { C_AREA.usable(p.cast()) } else { unsafe { __real_malloc_usable_size(p) } }
    }

    /// Whichever name frees it: newlib's `fclose` frees with `_free_r` what `setvbuf` took with `malloc`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __wrap__free_r(reent: *mut c_void, p: *mut c_void) {
        if C_AREA.contains(p.cast()) { unsafe { C_AREA.free_counted(p.cast()) } } else { unsafe { __real__free_r(reent, p) } }
    }

    /// A null `p` goes to the real one too: its `_malloc_r` is the real allocator, and so stays the block.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __wrap__realloc_r(reent: *mut c_void, p: *mut c_void, size: usize) -> *mut c_void {
        if C_AREA.contains(p.cast()) {
            return unsafe { moved(p, size) };
        }
        unsafe { __real__realloc_r(reent, p, size) }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn __wrap__malloc_usable_size_r(reent: *mut c_void, p: *mut c_void) -> usize {
        if C_AREA.contains(p.cast()) { C_AREA.usable(p.cast()) } else { unsafe { __real__malloc_usable_size_r(reent, p) } }
    }

    /// `mem-log`: who stays in the main heap. These only count, and hand on: what they return is a real chunk.
    #[cfg(feature = "mem-log")]
    mod counted {
        use super::super::callers::{return_address, TO_HEAP};
        use std::ffi::c_void;

        unsafe extern "C" {
            fn __real__malloc_r(reent: *mut c_void, size: usize) -> *mut c_void;
            fn __real__calloc_r(reent: *mut c_void, count: usize, size: usize) -> *mut c_void;
            fn __real_memalign(align: usize, size: usize) -> *mut c_void;
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn __wrap__malloc_r(reent: *mut c_void, size: usize) -> *mut c_void {
            TO_HEAP.note(return_address(), size);
            unsafe { __real__malloc_r(reent, size) }
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn __wrap__calloc_r(reent: *mut c_void, count: usize, size: usize) -> *mut c_void {
            TO_HEAP.note(return_address(), count.saturating_mul(size));
            unsafe { __real__calloc_r(reent, count, size) }
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn __wrap_memalign(align: usize, size: usize) -> *mut c_void {
            TO_HEAP.note(return_address(), size);
            unsafe { __real_memalign(align, size) }
        }
    }

    #[cfg(feature = "mem-log")]
    pub fn anchor() -> usize {
        __wrap_malloc as *const () as usize
    }
}

/// A `CHUNKS` log line (`mem-log`): what keeps the small blocks' chunks in use, which cut the main heap.
pub fn chunk_census(label: &str) {
    #[cfg(feature = "mem-log")]
    {
        let c = slab::census();
        let mut classes: Vec<(usize, u32)> = c.by_class.iter().copied().enumerate().filter(|(_, n)| *n > 0).collect();
        classes.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let top: Vec<String> = classes.iter().take(8).map(|(class, n)| format!("{}:{n}", class_size(*class))).collect();
        let tags: Vec<String> = tags::NAMES.iter().zip(c.by_tag).filter(|(_, n)| *n > 0).map(|(name, n)| format!("{name}={n}")).collect();
        println!(
            "CHUNKS {label}: {} in use, {} pages of {}, {} blocks, {:.2} MB; chunks holding <=4 blocks {}, <=32 {}, <=256 {}, more {}; by thread {}; by size {}",
            c.chunks,
            c.pages,
            c.chunks * 16,
            c.live,
            c.bytes as f64 / 1e6,
            c.by_live[0],
            c.by_live[1],
            c.by_live[2],
            c.by_live[3],
            tags.join(" "),
            top.join(" ")
        );
    }
    #[cfg(not(feature = "mem-log"))]
    let _ = label;
}

/// Bytes of the small blocks' region in use (whole pages).
pub fn small_region_used() -> usize {
    slab::bytes_used()
}

/// Chunks of the small blocks' region in use, and how many were ever taken.
pub fn small_chunks() -> (usize, usize) {
    slab::chunks()
}

/// A new small block: from the region, or from `malloc` when it is full.
///
/// Only the threads working for the world take the region's chunks: their blocks go with it. After 5.5 minutes in DownTown
/// 10 chunks outlived the world, and the main thread's few blocks held them: 1,470 of the 1,520 left, 0.34 MB keeping 10
/// MiB scattered through the main heap. The other threads' blocks past the fixed part go to the game's area.
#[inline]
unsafe fn small_new(slot: usize) -> *mut u8 {
    let world = WORLD_THREAD.try_with(Cell::get).unwrap_or(false);
    let p = slab::alloc(slot % CLASSES, world);
    if !p.is_null() {
        return p;
    }
    if !world {
        let p = GAME_AREA.alloc(slot_layout(slot));
        if !p.is_null() {
            return p;
        }
    }
    unsafe { System.alloc(slot_layout(slot)) }
}

#[inline]
unsafe fn small_free(p: *mut u8, slot: usize) {
    if slab::contains(p) {
        unsafe { slab::free(p) }
    } else if GAME_AREA.contains(p) {
        unsafe { GAME_AREA.free(p, slot_size(slot)) }
    } else {
        unsafe { System.dealloc(p, slot_layout(slot)) }
    }
}

struct Lists {
    head: [Cell<*mut u8>; SLOTS],
    count: [Cell<u32>; SLOTS],
    /// The last [`FLUSH_WANTED`] this thread acted on.
    flushed: Cell<u32>,
}

/// Raised by [`flush_every_thread`]: each thread gives back the blocks it keeps at its next small free.
static FLUSH_WANTED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Empties a thread's lists: the blocks go back to the region, where each could hold a page in use.
fn give_back(l: &Lists) {
    for slot in 0..SLOTS {
        let mut p = l.head[slot].replace(ptr::null_mut());
        l.count[slot].set(0);
        while !p.is_null() {
            // SAFETY: a free block's first word is the next one; these are ours and in no other list.
            let next = unsafe { *(p as *mut *mut u8) };
            SMALL_HELD.fetch_sub(slot_size(slot), Ordering::Relaxed);
            unsafe { small_free(p, slot) };
            p = next;
        }
    }
}

/// Before a world loads: every thread gives back what it keeps, this one now, the others at their next small free. Not
/// only the thread that loads: the main thread's and the audio's lists hold blocks of the last world too.
pub fn flush_every_thread() {
    FLUSH_WANTED.fetch_add(1, Ordering::Relaxed);
    flush_thread_cache();
}

/// Bytes of cached blocks returned to `malloc` by threads that ended (the `HEAP`
/// line: shows that the thread-local destructors run on the device).
static RETURNED: AtomicUsize = AtomicUsize::new(0);

pub fn returned_at_exit() -> usize {
    RETURNED.load(Ordering::Relaxed)
}

/// A thread that ends gives its cached blocks back. They used to stay held: every
/// collision window is built on a thread of its own, and each build left up to a megabyte of
/// small blocks behind (DownTown's small blocks grew 83 -> 116 MB in 100 s of riding, 48
/// windows). Blocks freed after this (other destructors of the thread) go straight to `malloc`:
/// `LISTS.try_with` fails once it is destroyed.
impl Drop for Lists {
    fn drop(&mut self) {
        let mut bytes = 0;
        for slot in 0..SLOTS {
            let mut p = self.head[slot].replace(ptr::null_mut());
            self.count[slot].set(0);
            while !p.is_null() {
                let next = unsafe { *(p as *mut *mut u8) };
                unsafe { small_free(p, slot) };
                bytes += slot_size(slot);
                p = next;
            }
        }
        SMALL_HELD.fetch_sub(bytes, Ordering::Relaxed);
        RETURNED.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// Gives back the blocks this thread keeps for itself: each can hold a page of the small blocks' region in use. Before
/// a world loads, when the last world's blocks have just been freed.
pub fn flush_thread_cache() {
    let _ = LISTS.try_with(give_back);
}

thread_local! {
    static LISTS: Lists = const {
        Lists { head: [const { Cell::new(ptr::null_mut()) }; SLOTS], count: [const { Cell::new(0) }; SLOTS], flushed: Cell::new(0) }
    };
}

pub struct Cached;

#[inline]
fn small(layout: &Layout) -> bool {
    layout.size() <= LARGE && layout.align() <= 16
}

unsafe impl GlobalAlloc for Cached {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !small(&layout) {
            let p = unsafe { large_alloc(layout) };
            if !p.is_null() {
                grew(layout.size());
            } else {
                failed(layout.size());
            }
            return p;
        }
        let slot = slot_of(&layout);
        #[cfg(feature = "mem-log")]
        tags::small(slot_size(slot) as isize);
        let hit = LISTS.try_with(|l| {
            let head = l.head[slot].get();
            if head.is_null() {
                return ptr::null_mut();
            }
            // The first word of a free block is the next one.
            l.head[slot].set(unsafe { *(head as *mut *mut u8) });
            l.count[slot].set(l.count[slot].get() - 1);
            head
        });
        match hit {
            Ok(p) if !p.is_null() => p,
            _ => {
                let p = unsafe { small_new(slot) };
                if !p.is_null() {
                    SMALL_HELD.fetch_add(slot_size(slot), Ordering::Relaxed);
                }
                p
            }
        }
    }

    #[inline]
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        if !small(&layout) {
            LARGE_LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            return unsafe { large_dealloc(p, layout) };
        }
        let slot = slot_of(&layout);
        #[cfg(feature = "mem-log")]
        tags::small(-(slot_size(slot) as isize));
        let kept = LISTS.try_with(|l| {
            let wanted = FLUSH_WANTED.load(Ordering::Relaxed);
            if l.flushed.get() != wanted {
                l.flushed.set(wanted);
                give_back(l);
            }
            if l.count[slot].get() >= keep(slot) {
                return false;
            }
            unsafe { *(p as *mut *mut u8) = l.head[slot].get() };
            l.head[slot].set(p);
            l.count[slot].set(l.count[slot].get() + 1);
            true
        });
        if kept != Ok(true) {
            SMALL_HELD.fetch_sub(slot_size(slot), Ordering::Relaxed);
            unsafe { small_free(p, slot) };
        }
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        if !small(&layout) && !small(&new_layout) {
            let q = unsafe { large_realloc(p, layout, new_size) };
            if !q.is_null() {
                LARGE_LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
                grew(new_size);
            } else {
                failed(new_size);
            }
            return q;
        }
        // Within the same size class the block already fits.
        if small(&layout) && small(&new_layout) && slot_of(&layout) == slot_of(&new_layout) {
            return p;
        }
        let q = unsafe { self.alloc(new_layout) };
        if !q.is_null() {
            unsafe {
                ptr::copy_nonoverlapping(p, q, layout.size().min(new_size));
                self.dealloc(p, layout);
            }
        }
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests share the region; the page counts it checks are of the whole region.
    static REGION_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
        REGION_TESTS.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn the_region_reuses_blocks_and_gives_empty_pages_to_any_class() {
        let _turn = one_at_a_time();
        // A class no other test uses much: 2048 + 64 bytes, 31 blocks a page.
        let class = class_of(2112);
        let blocks: Vec<*mut u8> = (0..31 * 40).map(|_| slab::alloc(class, true)).collect();
        assert!(blocks.iter().all(|p| !p.is_null() && slab::contains(*p) && p.addr() % 16 == 0));
        let unique: std::collections::HashSet<usize> = blocks.iter().map(|p| p.addr()).collect();
        assert_eq!(unique.len(), blocks.len());
        for w in blocks.windows(2) {
            assert!(w[0].addr().abs_diff(w[1].addr()) >= class_size(class), "blocks overlap");
        }
        unsafe { blocks.iter().for_each(|p| ptr::write_bytes(*p, 0x5A, class_size(class))) };
        unsafe { blocks.iter().for_each(|p| assert_eq!(**p, 0x5A)) };
        let used = slab::bytes_used();
        unsafe { blocks.iter().for_each(|p| slab::free(*p)) };
        assert!(slab::bytes_used() + 40 * (64 << 10) <= used + (64 << 10), "the pages came back");
        // Another class takes those pages: the region does not grow.
        let other = class_of(3008);
        let before = slab::bytes_used();
        let more: Vec<*mut u8> = (0..21 * 20).map(|_| slab::alloc(other, true)).collect();
        assert!(more.iter().all(|p| !p.is_null()));
        assert!(slab::bytes_used() <= before + 21 * (64 << 10));
        unsafe { more.iter().for_each(|p| slab::free(*p)) };
    }

    #[test]
    fn threads_allocating_and_freeing_each_others_blocks_lose_nothing() {
        let _turn = one_at_a_time();
        let a = Cached;
        let (tx, rx) = std::sync::mpsc::channel::<(usize, usize)>();
        let makers: Vec<_> = (0..4usize)
            .map(|t| {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    for i in 0..20_000usize {
                        let size = 16 + (i * 37 + t) % 4000;
                        let layout = Layout::from_size_align(size, 8).unwrap();
                        let p = unsafe { Cached.alloc(layout) };
                        assert!(!p.is_null());
                        unsafe { ptr::write_bytes(p, (size % 251) as u8, size) };
                        tx.send((p.addr(), size)).unwrap();
                    }
                })
            })
            .collect();
        drop(tx);
        // One thread frees all of them, checking that nothing overwrote them.
        for (addr, size) in rx {
            let p = ptr::without_provenance_mut::<u8>(addr);
            let bytes = unsafe { std::slice::from_raw_parts(p, size) };
            assert!(bytes.iter().all(|b| *b == (size % 251) as u8));
            unsafe { a.dealloc(p, Layout::from_size_align(size, 8).unwrap()) };
        }
        makers.into_iter().for_each(|m| m.join().unwrap());
    }

    #[test]
    fn past_the_fixed_region_chunks_come_and_go() {
        let _turn = one_at_a_time();
        let class = class_of(4096);
        let in_a_page = (64 << 10) / class_size(class);
        // Everything fixed in use, then 5 chunks' worth more.
        let count = (slab::REGION / (64 << 10) + 5 * 16) * in_a_page;
        let blocks: Vec<*mut u8> = (0..count).map(|_| slab::alloc(class, true)).collect();
        assert!(blocks.iter().all(|p| !p.is_null() && slab::contains(*p) && p.addr() % 16 == 0));
        let unique: std::collections::HashSet<usize> = blocks.iter().map(|p| p.addr()).collect();
        assert_eq!(unique.len(), count);
        assert!(slab::chunks().0 >= 5, "{:?}", slab::chunks());
        for (i, p) in blocks.iter().enumerate() {
            unsafe { ptr::write_bytes(*p, (i % 251) as u8, class_size(class)) };
        }
        for (i, p) in blocks.iter().enumerate() {
            let bytes = unsafe { std::slice::from_raw_parts(*p, class_size(class)) };
            assert!(bytes.iter().all(|b| *b == (i % 251) as u8), "block {i}");
        }
        let (in_use, taken) = slab::chunks();
        unsafe { blocks.iter().for_each(|p| slab::free(*p)) };
        assert_eq!(slab::chunks().0, 0, "the chunks went back ({in_use} in use, {taken} taken)");
        assert!(!slab::contains(blocks[count - 1]), "a chunk given back is not in the region");
        // And they come again.
        let again: Vec<*mut u8> = (0..count).map(|_| slab::alloc(class, true)).collect();
        assert!(again.iter().all(|p| !p.is_null()));
        assert!(slab::chunks().0 >= 5);
        unsafe { again.iter().for_each(|p| slab::free(*p)) };
        assert_eq!(slab::chunks().0, 0);
    }

    /// A chunk stays in use only for what nothing else has room for: with room in the fixed region, blocks go there, and a
    /// chunk that empties goes back.
    #[test]
    fn blocks_go_to_a_chunk_last() {
        let _turn = one_at_a_time();
        let class = class_of(4096);
        let in_a_page = (64 << 10) / class_size(class);
        let count = (slab::REGION / (64 << 10) + 2 * 16) * in_a_page;
        let blocks: Vec<*mut u8> = (0..count).map(|_| slab::alloc(class, true)).collect();
        assert!(slab::chunks().0 >= 2);
        assert!(slab::alloc(class, false).is_null(), "without chunks the fixed part is all there is");
        // A block leaves a fixed page and one leaves a chunk's page: the next goes to the fixed one, wherever the free came.
        let fixed = *blocks.iter().find(|p| slab::in_fixed(**p)).unwrap();
        let chunked = *blocks.iter().rev().find(|p| !slab::in_fixed(**p)).unwrap();
        unsafe {
            slab::free(fixed);
            slab::free(chunked);
        }
        let next = slab::alloc(class, true);
        assert_eq!(next, fixed, "the fixed region's room comes first");
        // Then the chunk's: no new chunk for a block that fits an old one.
        let (in_use, _) = slab::chunks();
        let last = slab::alloc(class, true);
        assert_eq!(last, chunked);
        assert_eq!(slab::chunks().0, in_use);
        unsafe {
            blocks.iter().filter(|p| **p != fixed && **p != chunked).for_each(|p| slab::free(*p));
            slab::free(next);
            slab::free(last);
        }
        assert_eq!(slab::chunks().0, 0, "all chunks went back");
    }

    #[test]
    fn a_thread_gives_back_its_blocks_at_its_next_free_when_asked() {
        let _turn = one_at_a_time();
        let a = Cached;
        let layout = Layout::from_size_align(1000, 8).unwrap();
        unsafe {
            let blocks: Vec<*mut u8> = (0..10).map(|_| a.alloc(layout)).collect();
            blocks.iter().for_each(|p| a.dealloc(*p, layout));
            let held = small_blocks();
            FLUSH_WANTED.fetch_add(1, Ordering::Relaxed);
            let p = a.alloc(layout);
            a.dealloc(p, layout);
            assert!(small_blocks() + 8 * class_size(class_of(1000)) <= held + class_size(class_of(1000)), "{} from {held}", small_blocks());
        }
    }

    /// Threads taking and returning blocks straight from the region, mixed classes, each block checked: a lost or doubly
    /// given block shows as another thread's pattern.
    #[test]
    fn the_region_holds_under_contention() {
        let _turn = one_at_a_time();
        let workers: Vec<_> = (0..8usize)
            .map(|t| {
                std::thread::spawn(move || {
                    let mut held: Vec<(*mut u8, usize, u8)> = Vec::new();
                    let mut x = t * 2654435761 + 1;
                    for i in 0..200_000usize {
                        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                        if held.len() > 2000 || (x >> 40) % 3 == 0 && !held.is_empty() {
                            let (p, class, mark) = held.swap_remove((x >> 20) % held.len());
                            let bytes = unsafe { std::slice::from_raw_parts(p, class_size(class)) };
                            assert!(bytes.iter().all(|b| *b == mark), "a block changed while held");
                            unsafe { slab::free(p) };
                        } else {
                            let class = (x >> 32) % CLASSES;
                            let p = slab::alloc(class, true);
                            assert!(!p.is_null() && slab::contains(p));
                            let mark = (i % 251) as u8;
                            unsafe { ptr::write_bytes(p, mark, class_size(class)) };
                            held.push((p, class, mark));
                        }
                    }
                    for (p, class, mark) in held {
                        let bytes = unsafe { std::slice::from_raw_parts(p, class_size(class)) };
                        assert!(bytes.iter().all(|b| *b == mark));
                        unsafe { slab::free(p) };
                    }
                    // Raw pointers do not cross threads; nothing is returned.
                })
            })
            .collect();
        workers.into_iter().for_each(|w| w.join().unwrap());
    }

    #[test]
    fn caches_keep_at_most_their_bytes_and_all_small_classes_keep_128() {
        assert_eq!(keep(slot_of(&Layout::from_size_align(16, 8).unwrap())), KEEP);
        assert_eq!(keep(slot_of(&Layout::from_size_align(512, 16).unwrap())), KEEP);
        assert_eq!(keep(slot_of(&Layout::from_size_align(4096, 8).unwrap())), 16);
        let worst: usize = (0..SLOTS).map(|s| keep(s) as usize * slot_size(s)).sum();
        assert!(worst < 10 << 20, "{worst}");
    }

    #[test]
    fn classes_cover_every_small_size_and_round_up() {
        for size in 1..=LARGE {
            let c = class_of(size);
            assert!(c < CLASSES, "{size}");
            assert!(class_size(c) >= size && (c == 0 || class_size(c - 1) < size), "{size} -> class {c}");
        }
        assert_eq!(class_size(CLASSES - 1), LARGE);
    }

    #[test]
    fn blocks_are_reused_by_their_thread_and_keep_their_contents() {
        let _turn = one_at_a_time();
        let a = Cached;
        unsafe {
            let layout = Layout::from_size_align(100, 8).unwrap();
            let p = a.alloc(layout);
            ptr::write_bytes(p, 0xAB, 100);
            a.dealloc(p, layout);
            let q = a.alloc(layout);
            assert_eq!(p, q, "the freed block comes back");
            a.dealloc(q, layout);
            // realloc across classes keeps the data; large blocks pass through.
            let p = a.alloc(Layout::from_size_align(40, 8).unwrap());
            ptr::write_bytes(p, 7, 40);
            let p = a.realloc(p, Layout::from_size_align(40, 8).unwrap(), 3000);
            assert!(std::slice::from_raw_parts(p, 40).iter().all(|&b| b == 7));
            let p = a.realloc(p, Layout::from_size_align(3000, 8).unwrap(), 70_000);
            assert!(std::slice::from_raw_parts(p, 40).iter().all(|&b| b == 7));
            let p = a.realloc(p, Layout::from_size_align(70_000, 8).unwrap(), 200_000);
            assert!(std::slice::from_raw_parts(p, 40).iter().all(|&b| b == 7));
            a.dealloc(p, Layout::from_size_align(200_000, 8).unwrap());
        }
    }

    #[test]
    fn a_thread_that_ends_returns_its_cached_blocks() {
        let _turn = one_at_a_time();
        let returned = returned_at_exit();
        std::thread::spawn(|| {
            let a = Cached;
            let layout = Layout::from_size_align(1000, 8).unwrap();
            unsafe {
                let blocks: Vec<*mut u8> = (0..10).map(|_| a.alloc(layout)).collect();
                for p in blocks {
                    a.dealloc(p, layout);
                }
            }
        })
        .join()
        .unwrap();
        assert!(returned_at_exit() >= returned + 10 * class_size(class_of(1000)));
    }

    #[test]
    fn small_blocks_keep_their_alignment_in_both_banks() {
        let _turn = one_at_a_time();
        let a = Cached;
        unsafe {
            for (size, align) in [(24, 8), (24, 16), (100, 4), (100, 16), (4000, 8), (4000, 16)] {
                let layout = Layout::from_size_align(size, align).unwrap();
                let blocks: Vec<*mut u8> = (0..20).map(|_| a.alloc(layout)).collect();
                assert!(blocks.iter().all(|p| p.addr() % align == 0), "{size}/{align}");
                for p in blocks {
                    a.dealloc(p, layout);
                }
            }
            // A 16-aligned block is not handed out from the 8-byte bank.
            let p = a.alloc(Layout::from_size_align(48, 8).unwrap());
            a.dealloc(p, Layout::from_size_align(48, 8).unwrap());
            let q = a.alloc(Layout::from_size_align(48, 16).unwrap());
            assert_eq!(q.addr() % 16, 0);
            a.dealloc(q, Layout::from_size_align(48, 16).unwrap());
        }
    }
}
