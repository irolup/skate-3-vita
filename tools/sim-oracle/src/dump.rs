//! Reader and comparer for the harness dump format (`harness/oracle_harness.rs`).
use std::path::Path;

const MAGIC: &[u8; 8] = b"SKODUMP1";
const BODY_WORDS: usize = 13;

#[derive(Clone, Debug, PartialEq)]
pub struct Tick {
    pub tick: u32,
    pub failed: u32,
    pub state: u32,
    /// Action then motion graph: (current, last, active (behaviour, instance) pairs).
    pub graphs: [(u32, u32, Vec<(u32, u32)>); 2],
    pub action_intents: u64,
    pub motion_intents: u64,
    /// family, active, name hash (low, high), output word, distance bits.
    /// Active grind name hash (2), animation chromosome (6), word 136, distance (b3c9679).
    pub grind: [u32; 10],
    /// present, x, y, z bits.
    pub camera: [u32; 4],
    /// Board bodies then skeleton bodies, BODY_WORDS each.
    pub bodies: Vec<u32>,
}

pub struct Dump {
    pub board_bodies: usize,
    pub skeleton_bodies: usize,
    pub ticks: Vec<Tick>,
}

pub fn read(path: &Path) -> Result<Dump, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() < 20 || &bytes[..8] != MAGIC {
        return Err(format!("{}: not a sim-oracle dump", path.display()));
    }
    let words: Vec<u32> = bytes[8..]
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let (version, board, skeleton) = (words[0], words[1] as usize, words[2] as usize);
    if version != 1 {
        return Err(format!("{}: dump version {version}", path.display()));
    }
    let mut at = 3;
    let mut ticks = Vec::new();
    let take = |at: &mut usize, n: usize| -> Result<Vec<u32>, String> {
        let v = words.get(*at..*at + n).ok_or("truncated dump")?.to_vec();
        *at += n;
        Ok(v)
    };
    while at < words.len() {
        let head = take(&mut at, 3)?;
        let mut graphs: [(u32, u32, Vec<(u32, u32)>); 2] = Default::default();
        for g in &mut graphs {
            let h = take(&mut at, 3)?;
            let pairs = take(&mut at, h[2] as usize * 2)?;
            *g = (h[0], h[1], pairs.chunks(2).map(|p| (p[0], p[1])).collect());
        }
        let intents = take(&mut at, 4)?;
        let grind: [u32; 10] = take(&mut at, 10)?.try_into().unwrap();
        let camera: [u32; 4] = take(&mut at, 4)?.try_into().unwrap();
        let bodies = take(&mut at, (board + skeleton) * BODY_WORDS)?;
        ticks.push(Tick {
            tick: head[0],
            failed: head[1],
            state: head[2],
            graphs,
            action_intents: u64::from(intents[0]) | u64::from(intents[1]) << 32,
            motion_intents: u64::from(intents[2]) | u64::from(intents[3]) << 32,
            grind,
            camera,
            bodies,
        });
    }
    Ok(Dump { board_bodies: board, skeleton_bodies: skeleton, ticks })
}

fn f(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// Largest position distance over the board bodies, in metres.
pub fn board_position_error(a: &Tick, b: &Tick, board_bodies: usize) -> f32 {
    (0..board_bodies)
        .map(|i| {
            let o = i * BODY_WORDS;
            let d: f32 = (0..3)
                .map(|k| f(a.bodies[o + k]) - f(b.bodies[o + k]))
                .map(|d| d * d)
                .sum();
            d.sqrt()
        })
        .fold(0.0, f32::max)
}

/// Bit equality, except that any two NaNs are equal: IEEE leaves a NaN's sign and
/// payload unspecified, and x86 and ARM propagate them differently.
fn same_float(a: u32, b: u32) -> bool {
    a == b || (f(a).is_nan() && f(b).is_nan())
}

/// Every dumped word bit-identical; float fields compare with [`same_float`].
fn same_tick(x: &Tick, y: &Tick) -> bool {
    x.tick == y.tick
        && discrete(x) == discrete(y)
        && x.action_intents == y.action_intents
        && x.motion_intents == y.motion_intents
        && x.grind[..9] == y.grind[..9]
        && same_float(x.grind[9], y.grind[9])
        && x.camera[0] == y.camera[0]
        && x.camera[1..].iter().zip(&y.camera[1..]).all(|(a, b)| same_float(*a, *b))
        && x.bodies.len() == y.bodies.len()
        && x.bodies.iter().zip(&y.bodies).all(|(a, b)| same_float(*a, *b))
}

/// Physical state and graph node sequence (no floats).
fn discrete(t: &Tick) -> (u32, u32, &(u32, u32, Vec<(u32, u32)>), &(u32, u32, Vec<(u32, u32)>)) {
    (t.failed, t.state, &t.graphs[0], &t.graphs[1])
}

pub struct Comparison {
    pub ticks: (usize, usize),
    pub first_raw_difference: Option<u32>,
    pub first_discrete_difference: Option<u32>,
    /// First discrete difference at or before tick 600 (the parity window).
    pub discrete_difference_by_600: Option<u32>,
    pub error_at_600: Option<f32>,
    pub max_error: f32,
    pub max_error_tick: u32,
}

pub fn compare(a: &Dump, b: &Dump) -> Result<Comparison, String> {
    if (a.board_bodies, a.skeleton_bodies) != (b.board_bodies, b.skeleton_bodies) {
        return Err("body counts differ".into());
    }
    let mut c = Comparison {
        ticks: (a.ticks.len(), b.ticks.len()),
        first_raw_difference: None,
        first_discrete_difference: None,
        discrete_difference_by_600: None,
        error_at_600: None,
        max_error: 0.0,
        max_error_tick: 0,
    };
    for (x, y) in a.ticks.iter().zip(&b.ticks) {
        if c.first_raw_difference.is_none() && !same_tick(x, y) {
            c.first_raw_difference = Some(x.tick);
        }
        if c.first_discrete_difference.is_none() && discrete(x) != discrete(y) {
            c.first_discrete_difference = Some(x.tick);
        }
        let e = board_position_error(x, y, a.board_bodies);
        if x.tick == 600 {
            c.error_at_600 = Some(e);
        }
        if e > c.max_error || e.is_nan() {
            c.max_error = e;
            c.max_error_tick = x.tick;
        }
    }
    c.discrete_difference_by_600 = c.first_discrete_difference.filter(|&t| t <= 600);
    if c.first_raw_difference.is_none() && a.ticks.len() != b.ticks.len() {
        c.first_raw_difference = Some(a.ticks.len().min(b.ticks.len()) as u32);
    }
    Ok(c)
}

/// States visited in order (run-length collapsed), for coverage reports.
pub fn state_runs(d: &Dump) -> Vec<(u32, u32)> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for t in &d.ticks {
        match runs.last() {
            Some(&(state, _)) if state == t.state => {}
            _ => runs.push((t.state, t.tick)),
        }
    }
    runs
}
