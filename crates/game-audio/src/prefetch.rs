//! Copy of the reference's `game_audio/native/prefetch.rs` (`2e16697`), tests not copied; on the Vita the
//! worker runs at low priority on core 0 (`set_thread_init`) and what it hides is
//! the memory card's latency (a bank is its program and its samples read from `world.pak`), not
//! decoding.
//!
//! World emitter banks read and decoded ahead of need, off the game thread (doc 11, "Emitter
//! bank prefetch"). Without it `Native::ensure_bank` read the `.abk` and decoded every WAV on the
//! game thread when a bank's first emitter started: 2.6–5.4 ms warm, up to 36 ms from a cold disk.
//!
//! - `emitters::update` asks for the banks of the emitters the listener is within
//!   [`AHEAD`] metres of (bounding sphere), nearest first, and drops the ones it moved [`EVICT`]
//!   metres away from before they were used. A map's emitter banks decode to 160–250 MiB in the
//!   districts, so only a ring around the listener is held (~20–30 MiB in the users' sessions).
//! - One worker thread (started on the first request) runs [`BankSource::load`], the same function
//!   the game thread runs without a prefetch, so the bank and PCM are identical.
//! - Only the data moves: `Runtime::load_bank` still runs in `ensure_bank`, at the emitter's start,
//!   so the runtime (bank ids, evaluator, mixer, random draws) sees exactly what it saw before.
//! - [`Prefetch::take`] at the start: done → the data; being decoded → wait for it (at most one
//!   bank's decode, never longer than loading it here); queued, failed or unknown → None, and the
//!   caller loads it on the game thread as before. A start is never delayed or deferred.
use std::collections::HashMap;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use crate::library::{BankPcm, BankSource};
use skate_audio::formats::Bank;

type Init = fn();
static THREAD_INIT: std::sync::OnceLock<Init> = std::sync::OnceLock::new();

/// Runs on the bank worker when it starts (the Vita pins it and lowers its priority).
pub fn set_thread_init(init: Init) {
    let _ = THREAD_INIT.set(init);
}

/// Prefetch a bank when the listener is this close to the edge of one of its emitters (m).
pub(crate) const AHEAD: f32 = 60.0;
/// Drop a prefetched, unused bank when the listener is this far from all its emitters (m).
pub(crate) const EVICT: f32 = 90.0;

enum Stage {
    Queued,
    Running,
    Done(Bank, BankPcm),
    /// The load failed or panicked, or the slot was dropped / taken before the worker reached it.
    Skip,
}

struct Slot {
    stage: Mutex<Stage>,
    ready: Condvar,
}

enum Job {
    Load(Arc<Slot>, BankSource),
    /// A dropped bank's data, freed on the worker (megabytes of PCM).
    Free(Box<(Bank, BankPcm)>),
}

/// The requested banks (stem → slot) and the worker's queue.
#[derive(Default)]
pub(crate) struct Prefetch {
    jobs: Option<Sender<Job>>,
    slots: HashMap<String, Arc<Slot>>,
    /// [`Prefetch::clear`] calls so far: a requester that remembers what it asked for (the world
    /// sources) knows its requests are gone when this changes.
    clears: u64,
}

impl Prefetch {

    /// Queue the bank for the worker (no-op when it is queued already).
    pub(crate) fn request(&mut self, source: BankSource) {
        if self.slots.contains_key(source.stem()) {
            return;
        }
        let slot = Arc::new(Slot { stage: Mutex::new(Stage::Queued), ready: Condvar::new() });
        let jobs = self.jobs.get_or_insert_with(spawn);
        let stem = source.stem().to_owned();
        if jobs.send(Job::Load(slot.clone(), source)).is_ok() {
            self.slots.insert(stem, slot);
        } else {
            // The worker is gone: loads stay on the game thread.
            self.jobs = None;
        }
    }

    /// The prefetched bank, if the worker has it or is decoding it (then wait for it). None: not
    /// requested, not started yet (the worker skips it now) or failed; the caller loads it itself.
    pub(crate) fn take(&mut self, stem: &str) -> Option<(Bank, BankPcm)> {
        let slot = self.slots.remove(stem)?;
        let Ok(stage) = slot.stage.lock() else { return None };
        let Ok(mut stage) = slot.ready.wait_while(stage, |s| matches!(s, Stage::Running)) else { return None };
        match std::mem::replace(&mut *stage, Stage::Skip) {
            Stage::Done(bank, pcm) => Some((bank, pcm)),
            _ => None,
        }
    }

    /// Whether the bank is done on the worker (not queued or running, not failed): `take` would not
    /// wait. skate_vita: the emitters start a node only when its bank is ready (a card read on the
    /// host thread would stall the pass).
    pub(crate) fn ready(&self, stem: &str) -> bool {
        self.slots.get(stem).is_some_and(|slot| slot.stage.try_lock().is_ok_and(|s| matches!(*s, Stage::Done(..))))
    }

    /// Forget a requested bank (queued: the worker skips it; done: its memory is freed).
    pub(crate) fn drop_bank(&mut self, stem: &str) {
        if let Some(slot) = self.slots.remove(stem) {
            self.free(skip(&slot));
        }
    }

    /// Forget every requested bank (map change).
    pub(crate) fn clear(&mut self) {
        self.clears += 1;
        let slots: Vec<_> = self.slots.drain().map(|(_, slot)| slot).collect();
        for slot in slots {
            self.free(skip(&slot));
        }
    }

    /// Whether the bank was requested and not taken or dropped since (tests).
    #[cfg(test)]
    pub(crate) fn contains(&self, stem: &str) -> bool {
        self.slots.contains_key(stem)
    }

    /// How many times [`Prefetch::clear`] ran (see the field).
    pub(crate) fn clears(&self) -> u64 {
        self.clears
    }

    /// Hand a dropped bank's data to the worker to free (dropped here when there is no worker).
    fn free(&self, data: Option<(Bank, BankPcm)>) {
        if let (Some(data), Some(jobs)) = (data, &self.jobs) {
            let _ = jobs.send(Job::Free(Box::new(data)));
        }
    }

}

/// Queued → skipped; done → its data returned (to be freed). A running load finishes on the
/// worker and is dropped there with the slot.
fn skip(slot: &Slot) -> Option<(Bank, BankPcm)> {
    let mut stage = slot.stage.lock().ok()?;
    if matches!(*stage, Stage::Running) {
        return None;
    }
    match std::mem::replace(&mut *stage, Stage::Skip) {
        Stage::Done(bank, pcm) => Some((bank, pcm)),
        _ => None,
    }
}

fn spawn() -> Sender<Job> {
    let (tx, rx) = channel::<Job>();
    let worker = std::thread::Builder::new().name("audio-bank-prefetch".into()).stack_size(256 * 1024).spawn(move || {
        if let Some(init) = THREAD_INIT.get() {
            init();
        }
        // Ends when the `Native` resource (the sender) is dropped.
        for job in rx {
            let (slot, source) = match job {
                Job::Load(slot, source) => (slot, source),
                Job::Free(data) => {
                    drop(data);
                    continue;
                }
            };
            {
                let Ok(mut stage) = slot.stage.lock() else { continue };
                if !matches!(*stage, Stage::Queued) {
                    continue;
                }
                *stage = Stage::Running;
            }
            // A failed or panicking load is left to the game thread, which then fails the same
            // way it always did.
            let result = std::panic::catch_unwind(|| source.load()).ok().and_then(Result::ok);
            let mut stage = match slot.stage.lock() {
                Ok(stage) => stage,
                Err(poisoned) => poisoned.into_inner(),
            };
            *stage = match result {
                Some((bank, pcm)) => Stage::Done(bank, pcm),
                None => Stage::Skip,
            };
            drop(stage);
            slot.ready.notify_all();
        }
    });
    if worker.is_err() {
        // No thread: the receiver is gone, so `request` falls back to game-thread loads.
        return channel::<Job>().0;
    }
    tx
}

