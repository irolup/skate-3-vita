//! District streaming benchmark (feature `stream-bench`): the game starts in each district in turn and the camera flies
//! a fixed loop through it (a rectangle inset in the district's tiles, a few metres above the spawn's height) at each
//! speed of `LEGS`, while the skater stands at the spawn. Per leg one `STREAM_BENCH` line: how often a tile within the
//! first level's range had nothing drawn (holes) or the wrong level (coarse), and the frame times. Then the app exits.
//! Run: `scripts/vita-run.py --features stream-bench` (the log ends at `STREAM_BENCH DONE`).
use glam::{Mat4, Vec3};
use std::time::Instant;

use crate::lab::Shared;
use crate::packs::WORLDS;

/// (district, speed in m/s, the streamer's lean reads) in order: each speed without, then with them (look-ahead on).
const LEGS: [(&str, f32, bool); 12] = [
    ("University", 12.0, false), ("University", 12.0, true), ("University", 24.0, false), ("University", 24.0, true),
    ("DownTown", 12.0, false), ("DownTown", 12.0, true), ("DownTown", 24.0, false), ("DownTown", 24.0, true),
    ("Industrial", 12.0, false), ("Industrial", 12.0, true), ("Industrial", 24.0, false), ("Industrial", 24.0, true),
];
/// The loop is inset this share of the tiles' extent on each side.
const INSET: f32 = 0.2;
/// Height above the spawn and the look-down angle (rad).
const HEIGHT: f32 = 4.0;
const PITCH: f32 = 0.15;
/// Seconds at the loop's start before the flight (the start's tiles load).
const SETTLE: f32 = 8.0;
/// One flight is one lap, at most this long (s).
const MAX_LAP: f32 = 60.0;

enum Phase {
    Load,
    Settle { since: Instant },
    Fly { distance: f32 },
    Done,
}

/// One distance band's sums over the frames.
#[derive(Clone, Copy, Default)]
struct Band {
    wanted: u64,
    holes: u64,
    hole_frames: u32,
    max_holes: u32,
    coarse: u64,
}

#[derive(Default)]
struct Stats {
    frames: u32,
    seconds: f32,
    bands: [Band; 3],
    over20: u32,
    over33: u32,
    worst_ms: f32,
    /// The loader's reads (count, bytes, µs) when the flight started.
    reads_at_start: (u64, u64, u64),
}

pub struct Bench {
    leg: usize,
    phase: Phase,
    /// The loop's corners (x, z) and height.
    corners: Vec<[f32; 2]>,
    height: f32,
    stats: Stats,
    /// The streamer starts from nothing before the leg (see `take_restart`).
    restart: bool,
}

impl Default for Bench {
    fn default() -> Self {
        Self { leg: 0, phase: Phase::Load, corners: Vec::new(), height: 0.0, stats: Stats::default(), restart: false }
    }
}

fn world(name: &str) -> usize {
    WORLDS.iter().position(|w| *w == name).unwrap_or(0)
}

/// The world the first leg loads (set before the simulation thread starts).
pub fn first_world() -> usize {
    world(LEGS[0].0)
}

impl Bench {
    /// Whether the streamer must drop its tiles and textures now (each leg's settle starts from nothing).
    pub fn take_restart(&mut self) -> bool {
        std::mem::take(&mut self.restart)
    }

    /// The streamer's lean reads for the current leg.
    pub fn lean(&self) -> bool {
        LEGS.get(self.leg).is_none_or(|l| l.2)
    }

    fn perimeter(&self) -> f32 {
        (0..self.corners.len()).map(|i| {
            let (a, b) = (self.corners[i], self.corners[(i + 1) % self.corners.len()]);
            ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
        }).sum()
    }

    /// The point and direction `distance` along the loop.
    fn at(&self, distance: f32) -> (Vec3, Vec3) {
        let mut d = distance.rem_euclid(self.perimeter().max(1.0));
        for i in 0..self.corners.len() {
            let (a, b) = (self.corners[i], self.corners[(i + 1) % self.corners.len()]);
            let (a, b) = (Vec3::new(a[0], self.height, a[1]), Vec3::new(b[0], self.height, b[1]));
            let side = a.distance(b);
            if d <= side || i + 1 == self.corners.len() {
                let dir = (b - a).normalize_or(Vec3::X);
                return (a + dir * d.min(side), dir);
            }
            d -= side;
        }
        (Vec3::ZERO, Vec3::X)
    }

    fn camera(&self, distance: f32) -> Mat4 {
        let (eye, dir) = self.at(distance);
        let target = eye + dir + Vec3::new(0.0, -PITCH.tan(), 0.0);
        Mat4::look_at_rh(eye, target, Vec3::Y).inverse()
    }

    /// Once per frame with the shared state locked: the next leg's load, the flight. Returns the camera to draw from
    /// (None: the game's own).
    pub fn update(&mut self, s: &mut Shared, frame_dt: f32) -> Option<Mat4> {
        let (name, speed, _) = *LEGS.get(self.leg)?;
        match self.phase {
            Phase::Load => {
                let w = world(name);
                if s.world != w {
                    s.world = w;
                    s.reload = true;
                    return None;
                }
                let ready = !s.loading && !s.reload && s.world_loaded == w && !s.frames.is_empty();
                let view = crate::packs::DISTRICT.lock().unwrap().clone();
                let (Some(view), true) = (view, ready) else { return None };
                let (xs, zs): (Vec<i32>, Vec<i32>) = view.tiles.keys().copied().unzip();
                let (x0, x1) = (*xs.iter().min()? as f32 * view.tile_size, (*xs.iter().max()? + 1) as f32 * view.tile_size);
                let (z0, z1) = (*zs.iter().min()? as f32 * view.tile_size, (*zs.iter().max()? + 1) as f32 * view.tile_size);
                let (ix, iz) = ((x1 - x0) * INSET, (z1 - z0) * INSET);
                self.corners = vec![[x0 + ix, z0 + iz], [x1 - ix, z0 + iz], [x1 - ix, z1 - iz], [x0 + ix, z1 - iz]];
                self.height = s.frames.back()?.current.root.translation.y + HEIGHT;
                println!("STREAM_BENCH {name}: loop {:.0} m at {:.0} m height, {} tiles of {:.0} m", self.perimeter(), self.height, view.tiles.len(), view.tile_size);
                self.phase = Phase::Settle { since: Instant::now() };
                self.restart = true;
                Some(self.camera(0.0))
            }
            Phase::Settle { since } => {
                if since.elapsed().as_secs_f32() >= SETTLE {
                    self.stats = Stats { reads_at_start: crate::district_stream::total_reads(), ..Stats::default() };
                    self.phase = Phase::Fly { distance: 0.0 };
                }
                Some(self.camera(0.0))
            }
            Phase::Fly { distance } => {
                let distance = distance + speed * frame_dt;
                let lap = self.perimeter();
                if distance >= lap || self.stats.seconds >= MAX_LAP {
                    let st = &self.stats;
                    let f = st.frames.max(1) as f64;
                    let bands: Vec<String> = ["<90 m", "90-160 m", "160-250 m"].iter().zip(&st.bands).map(|(label, b)| {
                        format!("{label}: {:.1} tiles, holes in {:.1}% of frames (mean {:.2}, max {}), coarse mean {:.2}", b.wanted as f64 / f, b.hole_frames as f64 * 100.0 / f, b.holes as f64 / f, b.max_holes, b.coarse as f64 / f)
                    }).collect();
                    let (n, bytes, us) = crate::district_stream::total_reads();
                    let (n0, bytes0, us0) = st.reads_at_start;
                    println!(
                        "STREAM_BENCH {name} {speed:.0} m/s lean reads {}: {:.0} m in {:.1} s, {} frames; {}; frames >20 ms {}, >33 ms {}, worst {:.1} ms; loader {} reads of {:.1} MiB in {:.1} s",
                        if LEGS[self.leg].2 { "on" } else { "off" },
                        distance.min(lap), st.seconds, st.frames, bands.join("; "), st.over20, st.over33, st.worst_ms, n - n0, (bytes - bytes0) as f64 / 1048576.0, (us - us0) as f64 / 1e6
                    );
                    self.leg += 1;
                    match LEGS.get(self.leg) {
                        Some((next, _, _)) if *next == name => {
                            self.phase = Phase::Settle { since: Instant::now() };
                            self.restart = true;
                        }
                        Some(_) => self.phase = Phase::Load,
                        None => {
                            println!("STREAM_BENCH DONE");
                            self.phase = Phase::Done;
                            s.quit = true;
                        }
                    }
                    return Some(self.camera(0.0));
                }
                self.phase = Phase::Fly { distance };
                Some(self.camera(distance))
            }
            Phase::Done => None,
        }
    }

    /// The frame's measurements (after the streamer's frame): its time and the near tiles' coverage.
    pub fn measure(&mut self, frame_dt: f32, coverage: [crate::district_stream::Coverage; 3]) {
        if !matches!(self.phase, Phase::Fly { .. }) {
            return;
        }
        let st = &mut self.stats;
        let ms = frame_dt * 1000.0;
        st.frames += 1;
        st.seconds += frame_dt;
        for (b, c) in st.bands.iter_mut().zip(coverage) {
            b.wanted += u64::from(c.wanted);
            b.holes += u64::from(c.holes);
            b.coarse += u64::from(c.coarse);
            b.hole_frames += u32::from(c.holes > 0);
            b.max_holes = b.max_holes.max(c.holes);
        }
        st.over20 += u32::from(ms > 20.0);
        st.over33 += u32::from(ms > 33.4);
        st.worst_ms = st.worst_ms.max(ms);
    }
}
