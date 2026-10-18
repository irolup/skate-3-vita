//! skate_vita diagnostic (feature `kernel-bench`): captures the inputs of the two
//! narrowphase entries during a real run, then times each phase on exactly those inputs.
//! Results are discarded through `black_box`; nothing here feeds the simulation.
use super::{
    ContactPrimitive, PrimitiveKind, PrimitivePairSettings, best_separating_direction,
    primitive_pair::separating_direction,
    primitive_query::{contact_points, geometry::pack, triangle_box_sat},
    primitive_pair_contacts, primitive_triangle_world_contacts, separating_axis_candidates,
    project_directions,
};
use crate::{
    math::Vector3,
    physics::collision::{Triangle, WorldContactSettings, world_separation_limit},
};
use std::hint::black_box;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

type World = (ContactPrimitive, Triangle, Vector3, WorldContactSettings);
type Pair = (ContactPrimitive, ContactPrimitive, PrimitivePairSettings);

const LIMIT: usize = 40_000;
static ENABLED: AtomicBool = AtomicBool::new(false);
static WORLD: Mutex<Vec<World>> = Mutex::new(Vec::new());
static PAIR: Mutex<Vec<Pair>> = Mutex::new(Vec::new());

pub fn set_capture(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub(super) fn capture_world(p: ContactPrimitive, t: Triangle, v: Vector3, s: WorldContactSettings) {
    if ENABLED.load(Ordering::Relaxed) {
        let mut w = WORLD.lock().unwrap();
        if w.len() < LIMIT {
            w.push((p, t, v, s));
        }
    }
}

pub(super) fn capture_pair(a: ContactPrimitive, b: ContactPrimitive, s: PrimitivePairSettings) {
    if ENABLED.load(Ordering::Relaxed) {
        let mut w = PAIR.lock().unwrap();
        if w.len() < LIMIT {
            w.push((a, b, s));
        }
    }
}

fn kind_name(p: &ContactPrimitive) -> &'static str {
    match p {
        ContactPrimitive::Sphere(_) => "sphere",
        ContactPrimitive::Capsule { .. } => "capsule",
        ContactPrimitive::Triangle(_) => "triangle",
        ContactPrimitive::RoundedBox { .. } => "box",
    }
}

/// Warm-cache cost in nanoseconds per call: blocks of 32 inputs, each block run
/// `rounds` times back to back, timing only the repeats after the first (so the
/// inputs are in L1, as the simulation's own working set mostly is).
fn time<T>(inputs: &[T], rounds: usize, mut f: impl FnMut(&T)) -> f64 {
    let mut total = 0u128;
    for block in inputs.chunks(32) {
        for input in block {
            f(input);
        }
        let start = Instant::now();
        for _ in 1..rounds {
            for input in block {
                f(input);
            }
        }
        total += start.elapsed().as_nanos();
    }
    total as f64 / (inputs.len() * (rounds - 1)).max(1) as f64
}

fn world_sat(i: &World) -> ([u32; 48], PrimitiveKind, [u32; 48], PrimitiveKind, ([u32; 4], [u32; 4])) {
    let (a, a_kind) = pack(i.0);
    let (b, b_kind) = pack(ContactPrimitive::Triangle(i.1));
    let sat = if matches!(a_kind, PrimitiveKind::Box) {
        let (separation, normal) = triangle_box_sat::triangle_box(&b, &a);
        (separation, normal.map(|v| v ^ 0x8000_0000))
    } else {
        best_separating_direction(&a, a_kind, &b, b_kind)
    };
    (a, a_kind, b, b_kind, sat)
}

#[inline(never)]
fn zero_stack<const N: usize>() {
    let a = [0u32; N];
    black_box(&a);
}

#[inline(never)]
fn copy_stack<const N: usize>(from: &[u32; N]) {
    let a = *black_box(from);
    black_box(&a);
}

fn memory_costs(out: &mut Vec<(String, usize, f64)>) {
    let dummy = [(); 4096];
    macro_rules! sizes {
        ($($n:literal),*) => {$(
            out.push((format!("mem.zero.{}B", $n * 4), 4096, time(&dummy, 17, |_| zero_stack::<$n>())));
            let source = [7u32; $n];
            out.push((format!("mem.copy.{}B", $n * 4), 4096, time(&dummy, 17, |_| copy_stack::<$n>(&source))));
        )*};
    }
    sizes!(16, 48, 64, 144, 256, 512, 1024);
    out.push(("mem.empty".into(), 4096, time(&dummy, 17, |_| black_box(()))));
}

/// (label, calls, ns per call) for every phase, grouped by moving-primitive kind.
pub fn run(rounds: usize) -> Vec<(String, usize, f64)> {
    set_capture(false);
    let world = std::mem::take(&mut *WORLD.lock().unwrap());
    let pair = std::mem::take(&mut *PAIR.lock().unwrap());
    let mut out = Vec::new();
    memory_costs(&mut out);
    for kind in ["sphere", "capsule", "triangle", "box", "all"] {
        let w: Vec<World> = world
            .iter()
            .filter(|i| kind == "all" || kind_name(&i.0) == kind)
            .copied()
            .collect();
        if w.is_empty() {
            continue;
        }
        let n = w.len();
        out.push((format!("world.{kind}.full"), n, time(&w, rounds, |i| {
            black_box(primitive_triangle_world_contacts(i.0, i.1, i.2, i.3));
        })));
        out.push((format!("world.{kind}.pack"), n, time(&w, rounds, |i| {
            black_box(pack(black_box(i.0)));
            black_box(pack(ContactPrimitive::Triangle(black_box(i.1))));
        })));
        out.push((format!("world.{kind}.pack+sat"), n, time(&w, rounds, |i| {
            black_box(world_sat(i));
        })));
        let packed: Vec<_> = w.iter().map(|i| (world_sat(i), *i)).collect();
        out.push((format!("world.{kind}.sat_only"), n, time(&packed, rounds, |(s, _)| {
            black_box(if matches!(s.1, PrimitiveKind::Box) {
                triangle_box_sat::triangle_box(black_box(&s.2), black_box(&s.0))
            } else {
                best_separating_direction(black_box(&s.0), s.1, black_box(&s.2), s.3)
            });
        })));
        out.push((format!("world.{kind}.candidates"), n, time(&packed, rounds, |(s, _)| {
            let mut directions = [[0; 4]; 16];
            black_box(separating_axis_candidates(&s.0, &s.2, &mut directions));
            black_box(directions);
        })));
        out.push((format!("world.{kind}.project"), n, time(&packed, rounds, |(s, _)| {
            let mut directions = [[0; 4]; 16];
            let count = separating_axis_candidates(&s.0, &s.2, &mut directions).max(1);
            let mut ai = [[0; 12]; 16];
            let mut bi = [[0; 12]; 16];
            project_directions(&s.0, s.1, &directions[..count], &mut ai);
            project_directions(&s.2, s.3, &directions[..count], &mut bi);
            black_box((ai, bi));
        })));
        out.push((format!("world.{kind}.points"), n, time(&packed, rounds, |(s, i)| {
            let limit = world_separation_limit(
                i.2,
                i.1.feature.normal,
                i.3.volume_padding,
                i.3.maximum_separating_distance,
            );
            black_box(contact_points(
                &s.0,
                s.1,
                &s.2,
                s.3,
                i.1,
                f32::from_bits(s.4.0[0]),
                s.4.1,
                limit,
                i.3,
            ));
        })));
    }
    for kind in ["sphere", "capsule", "triangle", "box", "all"] {
        let p: Vec<Pair> = pair
            .iter()
            .filter(|i| kind == "all" || kind_name(&i.0) == kind)
            .copied()
            .collect();
        if p.is_empty() {
            continue;
        }
        let n = p.len();
        out.push((format!("pair.{kind}.full"), n, time(&p, rounds, |i| {
            black_box(primitive_pair_contacts(i.0, i.1, i.2));
        })));
        out.push((format!("pair.{kind}.pack+sat"), n, time(&p, rounds, |i| {
            let (a, a_kind) = pack(i.0);
            let (b, b_kind) = pack(i.1);
            black_box(separating_direction(&a, a_kind, &b, b_kind));
        })));
    }
    out
}
