//! skate_vita addition (retail-audio S1): foreground card reads and the background checks that
//! must not slow them down.
//!
//! The Vita's memory card reads about 13 MB/s. At boot and at a world change the loaders read
//! the packs from it (`vfs` entries on disk) while the background hash checks read the same
//! packs and the 100-250 MB district packs end to end; together they made the retail menu take
//! 14 s instead of 1.7 s. A loader's read holds a [`Foreground`] guard; a background check calls
//! [`wait_background`] before each piece it reads and stays out of the way while a foreground
//! read runs or has just finished.
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

static ACTIVE: AtomicU32 = AtomicU32::new(0);
static LAST_END_MS: AtomicU64 = AtomicU64::new(0);
/// A background check resumes when no foreground read ended in the last 150 ms.
const QUIET_MS: u64 = 150;

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// A foreground read in progress (dropped when it ends).
pub struct Foreground(());

/// Marks a foreground card read until the guard is dropped.
pub fn foreground() -> Foreground {
    ACTIVE.fetch_add(1, Ordering::Relaxed);
    Foreground(())
}

impl Drop for Foreground {
    fn drop(&mut self) {
        LAST_END_MS.store(now_ms(), Ordering::Relaxed);
        ACTIVE.fetch_sub(1, Ordering::Relaxed);
    }
}

/// For a background check, before it reads its next piece: waits while a foreground read runs or
/// ended less than 150 ms ago, for at most `max` (the check goes on slowly rather than starving).
pub fn wait_background(max: Duration) {
    let started = Instant::now();
    while started.elapsed() < max {
        let recent = now_ms().saturating_sub(LAST_END_MS.load(Ordering::Relaxed)) < QUIET_MS;
        if ACTIVE.load(Ordering::Relaxed) == 0 && !(recent && LAST_END_MS.load(Ordering::Relaxed) != 0) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// skate_vita (optimization phase): card reads by source since the last [`take_accounts`] (count, bytes, µs, slowest
/// µs), for the device log.
static ACCOUNTS: std::sync::Mutex<Vec<(String, [u64; 4])>> = std::sync::Mutex::new(Vec::new());

/// Counts one card read of `bytes` that took `us` for `source`.
pub fn account(source: &str, bytes: usize, us: u64) {
    let Ok(mut accounts) = ACCOUNTS.lock() else { return };
    let slot = match accounts.iter().position(|(s, _)| s == source) {
        Some(i) => &mut accounts[i].1,
        None => {
            accounts.push((source.to_owned(), [0; 4]));
            &mut accounts.last_mut().unwrap().1
        }
    };
    slot[0] += 1;
    slot[1] += bytes as u64;
    slot[2] += us;
    slot[3] = slot[3].max(us);
}

/// The reads counted since the last call, busiest (by time) first; the counts restart.
pub fn take_accounts() -> Vec<(String, [u64; 4])> {
    let mut out = ACCOUNTS.lock().map(|mut a| a.drain(..).collect::<Vec<_>>()).unwrap_or_default();
    out.sort_by(|a, b| b.1[2].cmp(&a.1[2]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_background_check_waits_for_a_foreground_read_and_then_goes() {
        wait_background(Duration::from_millis(500));
        let guard = foreground();
        let started = Instant::now();
        wait_background(Duration::from_millis(100));
        assert!(started.elapsed() >= Duration::from_millis(100), "returned during a foreground read");
        drop(guard);
        let started = Instant::now();
        wait_background(Duration::from_secs(2));
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(100) && waited < Duration::from_millis(400), "{waited:?}");
    }
}
