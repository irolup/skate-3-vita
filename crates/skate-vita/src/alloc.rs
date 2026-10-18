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

fn grew(by: usize) {
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

/// Tags the current thread for the `mem-log` breakdown (one of `tags::NAMES`; a no-op without
/// the feature).
pub fn tag_thread(kind: &str) {
    #[cfg(feature = "mem-log")]
    tags::set(kind);
    #[cfg(not(feature = "mem-log"))]
    let _ = kind;
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

/// A large block of this layout carries the tag header (`mem-log`).
#[cfg(feature = "mem-log")]
#[inline]
fn tagged(layout: &Layout) -> bool {
    layout.align() <= tags::HEADER
}

#[cfg(feature = "mem-log")]
unsafe fn large_alloc(layout: Layout) -> *mut u8 {
    if !tagged(&layout) {
        return unsafe { System.alloc(layout) };
    }
    let base = unsafe { System.alloc(Layout::from_size_align_unchecked(layout.size() + tags::HEADER, layout.align())) };
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
    if !tagged(&layout) {
        return unsafe { System.dealloc(p, layout) };
    }
    let base = unsafe { p.sub(tags::HEADER) };
    tags::LARGE[unsafe { *base } as usize].fetch_sub(layout.size() as isize, Ordering::Relaxed);
    unsafe { System.dealloc(base, Layout::from_size_align_unchecked(layout.size() + tags::HEADER, layout.align())) }
}

#[cfg(feature = "mem-log")]
unsafe fn large_realloc(p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    if !tagged(&layout) {
        return unsafe { System.realloc(p, layout, new_size) };
    }
    let base = unsafe { p.sub(tags::HEADER) };
    let tag = unsafe { *base } as usize;
    let q = unsafe { System.realloc(base, Layout::from_size_align_unchecked(layout.size() + tags::HEADER, layout.align()), new_size + tags::HEADER) };
    if q.is_null() {
        return q;
    }
    tags::LARGE[tag].fetch_add(new_size as isize - layout.size() as isize, Ordering::Relaxed);
    unsafe { q.add(tags::HEADER) }
}

#[cfg(not(feature = "mem-log"))]
#[inline]
unsafe fn large_alloc(layout: Layout) -> *mut u8 {
    unsafe { System.alloc(layout) }
}

#[cfg(not(feature = "mem-log"))]
#[inline]
unsafe fn large_dealloc(p: *mut u8, layout: Layout) {
    unsafe { System.dealloc(p, layout) }
}

#[cfg(not(feature = "mem-log"))]
#[inline]
unsafe fn large_realloc(p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    unsafe { System.realloc(p, layout, new_size) }
}

/// A large allocation failed: which thread asked and for how much (the abort message that
/// follows has neither). Small allocations still work at that point, so printing is safe.
#[cold]
fn failed(size: usize) {
    static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !ONCE.swap(true, Ordering::Relaxed) {
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
/// The most blocks a thread keeps per class (a thread that stops leaves them cached).
const KEEP: u32 = 128;

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

struct Lists {
    head: [Cell<*mut u8>; SLOTS],
    count: [Cell<u32>; SLOTS],
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
                unsafe { System.dealloc(p, slot_layout(slot)) };
                bytes += slot_size(slot);
                p = next;
            }
        }
        SMALL_HELD.fetch_sub(bytes, Ordering::Relaxed);
        RETURNED.fetch_add(bytes, Ordering::Relaxed);
    }
}

thread_local! {
    static LISTS: Lists = const {
        Lists { head: [const { Cell::new(ptr::null_mut()) }; SLOTS], count: [const { Cell::new(0) }; SLOTS] }
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
                let p = unsafe { System.alloc(slot_layout(slot)) };
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
            if l.count[slot].get() >= KEEP {
                return false;
            }
            unsafe { *(p as *mut *mut u8) = l.head[slot].get() };
            l.head[slot].set(p);
            l.count[slot].set(l.count[slot].get() + 1);
            true
        });
        if kept != Ok(true) {
            SMALL_HELD.fetch_sub(slot_size(slot), Ordering::Relaxed);
            unsafe { System.dealloc(p, slot_layout(slot)) };
        }
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        if !small(&layout) && !small(&new_layout) {
            let q = unsafe { large_realloc(p, layout, new_size) };
            if !q.is_null() {
                LARGE_LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
                grew(new_size);
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
