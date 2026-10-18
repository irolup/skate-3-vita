//! Minimal tracing subscriber: sums the time spent inside each span name (the
//! skate-sim stage spans) for the profile. Info/debug events are ignored,
//! so no logging lands inside a measurement; warnings and errors are printed.
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

#[derive(Default)]
struct Totals {
    time: Duration,
    count: u64,
}

pub struct Profiler {
    next: AtomicU64,
    names: Mutex<HashMap<u64, &'static str>>,
    open: Mutex<HashMap<u64, Instant>>,
    totals: Mutex<HashMap<&'static str, Totals>>,
}

impl Profiler {
    pub fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
            names: Mutex::new(HashMap::new()),
            open: Mutex::new(HashMap::new()),
            totals: Mutex::new(HashMap::new()),
        }
    }

    /// (span name, total time, entries) since the last call; resets the totals.
    pub fn take(&self) -> Vec<(&'static str, Duration, u64)> {
        let mut totals = self.totals.lock().unwrap();
        let mut v: Vec<_> = totals.drain().map(|(k, t)| (k, t.time, t.count)).collect();
        v.sort_by_key(|e| e.0);
        v
    }
}

impl Subscriber for &'static Profiler {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.is_span() || *metadata.level() <= Level::WARN
    }
    fn new_span(&self, attrs: &Attributes<'_>) -> Id {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.names.lock().unwrap().insert(id, attrs.metadata().name());
        Id::from_u64(id)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        println!("BENCH LOG {} {}", event.metadata().level(), event.metadata().target());
    }
    fn enter(&self, id: &Id) {
        self.open.lock().unwrap().insert(id.into_u64(), Instant::now());
    }
    fn exit(&self, id: &Id) {
        let now = Instant::now();
        let Some(start) = self.open.lock().unwrap().remove(&id.into_u64()) else { return };
        let Some(&name) = self.names.lock().unwrap().get(&id.into_u64()) else { return };
        let mut totals = self.totals.lock().unwrap();
        let t = totals.entry(name).or_default();
        t.time += now - start;
        t.count += 1;
    }
    fn try_close(&self, id: Id) -> bool {
        self.names.lock().unwrap().remove(&id.into_u64());
        true
    }
}
