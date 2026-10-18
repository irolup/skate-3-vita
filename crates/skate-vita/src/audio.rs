//! Procedural game sounds (a rolling loop whose pitch and volume follow
//! speed; pop, land, grind and slide, bail). Everything is synthesized here from noise
//! and simple oscillators, so no sample files (retail or third-party) are involved.
//!
//! The simulation thread sets [`Params`] and queues [`Event`]s every tick; the audio
//! thread pulls 48 kHz stereo blocks with [`Synth::fill`].

pub const RATE: f32 = 48_000.0;

/// Continuous sounds, updated every tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Params {
    /// Board speed in m/s while the wheels roll on the ground (0 in the air).
    pub rolling: f32,
    /// Board speed while grinding a rail (metal).
    pub grinding: f32,
    /// Board speed while sliding (board on a ledge or rail lip, or a powerslide).
    pub sliding: f32,
    /// Master volume 0..1.
    pub volume: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Ollie / takeoff.
    Pop,
    /// Landing; strength 0..1 (from the fall speed).
    Land(f32),
    /// Wipeout.
    Bail,
    /// Session marker placed / refused / returning.
    MarkerSet,
    MarkerDenied,
    MarkerReturn,
}

/// One-pole low-pass filter.
#[derive(Clone, Copy, Default)]
struct LowPass {
    y: f32,
}
impl LowPass {
    fn step(&mut self, x: f32, cutoff: f32) -> f32 {
        let a = 1.0 - (-core::f32::consts::TAU * cutoff / RATE).exp();
        self.y += a * (x - self.y);
        self.y
    }
}

/// State-variable band-pass filter.
#[derive(Clone, Copy, Default)]
struct BandPass {
    low: f32,
    band: f32,
}
impl BandPass {
    fn step(&mut self, x: f32, centre: f32, q: f32) -> f32 {
        let f = 2.0 * (core::f32::consts::PI * centre / RATE).sin();
        let high = x - self.low - self.band / q;
        self.band += f * high;
        self.low += f * self.band;
        self.band
    }
}

/// A decaying one-shot: a pitched thump with a noise click.
#[derive(Clone, Copy)]
struct Shot {
    age: f32,
    length: f32,
    freq_start: f32,
    freq_end: f32,
    tone: f32,
    noise: f32,
    phase: f32,
}

pub struct Synth {
    seed: u32,
    params: Params,
    /// Smoothed copies of the params (no zipper noise between ticks).
    rolling: f32,
    grinding: f32,
    sliding: f32,
    roll_low: LowPass,
    roll_rumble: LowPass,
    roll_phase: f32,
    grind_band: [BandPass; 2],
    grind_phase: [f32; 2],
    slide_band: BandPass,
    shots: Vec<Shot>,
}

impl Default for Synth {
    fn default() -> Self {
        Self {
            seed: 0x1234_5678,
            params: Params { volume: 0.8, ..Params::default() },
            rolling: 0.0,
            grinding: 0.0,
            sliding: 0.0,
            roll_low: LowPass::default(),
            roll_rumble: LowPass::default(),
            roll_phase: 0.0,
            grind_band: [BandPass::default(); 2],
            grind_phase: [0.0; 2],
            slide_band: BandPass::default(),
            shots: Vec::with_capacity(8),
        }
    }
}

impl Synth {
    pub fn set(&mut self, params: Params) {
        self.params = params;
    }

    pub fn trigger(&mut self, event: Event) {
        let shot = match event {
            Event::Pop => Shot { age: 0.0, length: 0.09, freq_start: 220.0, freq_end: 110.0, tone: 0.55, noise: 0.45, phase: 0.0 },
            Event::Land(s) => {
                let s = s.clamp(0.2, 1.0);
                Shot { age: 0.0, length: 0.08 + 0.1 * s, freq_start: 95.0, freq_end: 45.0, tone: 0.7 * s, noise: 0.35 * s, phase: 0.0 }
            }
            Event::Bail => Shot { age: 0.0, length: 0.35, freq_start: 80.0, freq_end: 35.0, tone: 0.8, noise: 0.6, phase: 0.0 },
            Event::MarkerSet => Shot { age: 0.0, length: 0.14, freq_start: 660.0, freq_end: 990.0, tone: 0.45, noise: 0.02, phase: 0.0 },
            Event::MarkerDenied => Shot { age: 0.0, length: 0.18, freq_start: 150.0, freq_end: 110.0, tone: 0.5, noise: 0.15, phase: 0.0 },
            Event::MarkerReturn => Shot { age: 0.0, length: 0.4, freq_start: 300.0, freq_end: 1300.0, tone: 0.35, noise: 0.1, phase: 0.0 },
        };
        if self.shots.len() < 8 {
            self.shots.push(shot);
        }
    }

    fn noise(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Fills interleaved stereo 16-bit frames.
    pub fn fill(&mut self, out: &mut [i16]) {
        let p = self.params;
        // ~20 ms smoothing towards the tick's values.
        let smooth = 1.0 - (-1.0 / (0.02 * RATE)).exp();
        for frame in out.chunks_exact_mut(2) {
            self.rolling += smooth * (p.rolling - self.rolling);
            self.grinding += smooth * (p.grinding - self.grinding);
            self.sliding += smooth * (p.sliding - self.sliding);
            let n = self.noise();
            let mut x = 0.0;

            // Rolling: low-passed noise whose brightness and level follow speed, over a
            // low rumble; urethane on concrete.
            if self.rolling > 0.05 {
                let v = (self.rolling / 10.0).min(1.0);
                let hiss = self.roll_low.step(n, 250.0 + 1800.0 * v);
                self.roll_phase = (self.roll_phase + (40.0 + 60.0 * v) / RATE).fract();
                let rumble = self.roll_rumble.step(n, 90.0) * 4.0 * (0.6 + 0.4 * (self.roll_phase * core::f32::consts::TAU).sin());
                x += (0.30 * hiss + 0.25 * rumble) * v.sqrt();
            }
            // Grind: metal on metal, two ringing partials over band-passed noise.
            if self.grinding > 0.05 {
                let v = (self.grinding / 8.0).clamp(0.3, 1.0);
                let scrape = self.grind_band[0].step(n, 2400.0, 3.0) + 0.6 * self.grind_band[1].step(n, 4100.0, 5.0);
                let mut ring = 0.0;
                for (i, f) in [1870.0, 3130.0].into_iter().enumerate() {
                    self.grind_phase[i] = (self.grind_phase[i] + f * (0.98 + 0.04 * v) / RATE).fract();
                    ring += (self.grind_phase[i] * core::f32::consts::TAU).sin();
                }
                x += v * (0.35 * scrape + 0.06 * ring * (0.5 + 0.5 * n.abs()));
            }
            // Slide: wood or concrete scrape, lower and duller.
            if self.sliding > 0.05 {
                let v = (self.sliding / 8.0).clamp(0.3, 1.0);
                x += 0.45 * v * self.slide_band.step(n, 750.0, 1.5);
            }
            // One-shots.
            for s in &mut self.shots {
                let t = s.age / s.length;
                let env = (1.0 - t).max(0.0).powi(2);
                let f = s.freq_start + (s.freq_end - s.freq_start) * t;
                s.phase = (s.phase + f / RATE).fract();
                let click = if s.age < 0.006 { 1.0 } else { 0.25 };
                x += env * (s.tone * (s.phase * core::f32::consts::TAU).sin() + s.noise * click * n);
                s.age += 1.0 / RATE;
            }
            self.shots.retain(|s| s.age < s.length);

            let y = (x * p.volume).tanh();
            let sample = (y * 30_000.0) as i16;
            frame[0] = sample;
            frame[1] = sample;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn energy(synth: &mut Synth, frames: usize) -> f64 {
        let mut buf = vec![0i16; frames * 2];
        synth.fill(&mut buf);
        buf.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / buf.len() as f64
    }

    #[test]
    fn silence_when_nothing_happens() {
        let mut s = Synth::default();
        assert_eq!(energy(&mut s, 4800), 0.0);
    }

    #[test]
    fn rolling_gets_louder_with_speed_and_stops_in_the_air() {
        let mut s = Synth::default();
        s.set(Params { rolling: 2.0, volume: 0.8, ..Params::default() });
        let slow = energy(&mut s, 9600);
        s.set(Params { rolling: 9.0, volume: 0.8, ..Params::default() });
        let fast = energy(&mut s, 9600);
        assert!(fast > slow * 1.5, "{slow} {fast}");
        s.set(Params { volume: 0.8, ..Params::default() });
        energy(&mut s, 9600);
        assert!(energy(&mut s, 4800) < slow * 0.01);
    }

    #[test]
    fn one_shots_decay_away_and_never_clip_into_overflow() {
        let mut s = Synth::default();
        s.trigger(Event::Bail);
        s.trigger(Event::Land(1.0));
        s.trigger(Event::Pop);
        s.set(Params { rolling: 10.0, grinding: 8.0, sliding: 8.0, volume: 1.0 });
        let mut buf = vec![0i16; 4800 * 2];
        s.fill(&mut buf);
        assert!(buf.iter().all(|&v| v.abs() <= 30_000));
        s.set(Params { volume: 1.0, ..Params::default() });
        energy(&mut s, 48_000);
        assert!(s.shots.is_empty());
        assert!(energy(&mut s, 4800) < 1.0);
    }
}

/// What the simulation thread hands the audio thread: the latest params and the
/// events since the last block (the audio thread locks this once per 5.3 ms block).
pub struct Feed {
    pub params: Params,
    pub events: Vec<Event>,
}
pub static FEED: std::sync::Mutex<Feed> = std::sync::Mutex::new(Feed {
    params: Params { rolling: 0.0, grinding: 0.0, sliding: 0.0, volume: 0.8 },
    events: Vec::new(),
});

/// Turns the skater's state each tick into sound (the reference has no skater audio).
/// State ids: skate-core `PhysicalStateId`.
#[derive(Default)]
pub struct Cues {
    previous: u32,
    previous_height: Option<f32>,
    /// Fastest fall (m/s, positive down) since leaving the ground.
    fall: f32,
}

fn grounded(id: u32) -> bool {
    (100..=103).contains(&id)
}
fn airborne(id: u32) -> bool {
    (200..=202).contains(&id)
}

impl Cues {
    /// One tick: state id, deck height (m) and board speed (m/s); `dt` the tick length.
    pub fn tick(&mut self, id: u32, height: f32, speed: f32, dt: f32, events: &mut Vec<Event>) -> (f32, f32, f32) {
        let vertical = self.previous_height.map_or(0.0, |h| (height - h) / dt);
        self.previous_height = Some(height);
        if airborne(id) {
            self.fall = self.fall.max(-vertical);
        }
        if grounded(self.previous) && airborne(id) {
            events.push(Event::Pop);
            self.fall = 0.0;
        }
        if airborne(self.previous) && (grounded(id) || (400..=405).contains(&id)) {
            events.push(Event::Land((self.fall / 7.0).clamp(0.2, 1.0)));
        }
        if id == 300 && self.previous != 300 {
            events.push(Event::Bail);
        }
        self.previous = id;
        let rolling = if grounded(id) && id != 101 { speed } else { 0.0 };
        // 50-50, 5-0, crooked-type grinds on the trucks: metal; board and tip slides,
        // darkslide and powerslide: a duller scrape.
        let grinding = if matches!(id, 401 | 403 | 404) { speed } else { 0.0 };
        let sliding = if matches!(id, 101 | 400 | 402 | 405) { speed } else { 0.0 };
        (rolling, grinding, sliding)
    }
}

/// The Audio setting: the retail board audio plays instead of the synth's board sounds.
static RETAIL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Set once the sound.pak loader has been started (it runs once per launch).
static LOADER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Applies the Audio setting. Retail starts loading sound.pak the first time it is chosen;
/// back to Synth only stops the retail render (its memory is freed at the next launch).
pub fn set_retail(on: bool) {
    RETAIL.store(on, std::sync::atomic::Ordering::Relaxed);
    #[cfg(target_os = "vita")]
    if on && !LOADER.swap(true, std::sync::atomic::Ordering::Relaxed) {
        let spawned = std::thread::Builder::new()
            .name("sound-host".into())
            .stack_size(2 * 1024 * 1024)
            .spawn(|| {
                crate::alloc::tag_thread("audio");
                // Core 0 at low priority: the main thread leaves most of core 0 idle, and core 2
                // is the audio render's and the collision window builder's.
                let _ = vita_platform::vita::pin_current_thread(0);
                vita_platform::vita::set_current_thread_priority(160);
                if let Err(e) = crate::retail_audio::load_and_run() {
                    println!("{}: {e}; the procedural sounds play instead", crate::retail_audio::PACK);
                }
            });
        if spawned.is_err() {
            println!("sound-host thread not started; the procedural sounds play instead");
        }
    }
}

/// Starts the audio thread: pulls the feed into the synthesizer every block. With the Audio
/// setting on Retail and `sound.pak` loaded (on a loader thread), the retail board audio
/// (`crate::retail_audio`) replaces the synth's board sounds; the synth keeps the
/// session-marker cues until those are ported.
#[cfg(target_os = "vita")]
pub fn start(retail_audio: bool) {
    let mut synth = Synth::default();
    let mut retail: Option<crate::retail_audio::Player> = None;
    let ok = vita_platform::vita::start_audio(move |out| {
        let on = RETAIL.load(std::sync::atomic::Ordering::Relaxed);
        if on && retail.is_none() {
            retail = crate::retail_audio::Player::take_ready();
        }
        let playing = on && retail.is_some();
        let volume = {
            let mut feed = FEED.lock().unwrap();
            let mut params = feed.params;
            if playing {
                (params.rolling, params.grinding, params.sliding) = (0.0, 0.0, 0.0);
            }
            synth.set(params);
            for e in feed.events.drain(..) {
                // The marker cues are the synth's until the retail front-end sounds are loaded.
                if !playing || (!crate::retail_audio::frontend_ready() && matches!(e, Event::MarkerSet | Event::MarkerDenied | Event::MarkerReturn)) {
                    synth.trigger(e);
                }
            }
            params.volume
        };
        synth.fill(out);
        if let (true, Some(player)) = (playing, &mut retail) {
            for (o, x) in out.iter_mut().zip(player.fill(AUDIO_FRAMES)) {
                *o = (f32::from(*o) + x * volume * 32767.0).clamp(-32768.0, 32767.0) as i16;
            }
        }
    });
    println!("audio: {}", if ok { "48 kHz stereo on core 2" } else { "unavailable" });
    if ok {
        set_retail(retail_audio);
    }
}

#[cfg(target_os = "vita")]
use vita_platform::vita::AUDIO_FRAMES;

#[cfg(test)]
mod cue_tests {
    use super::*;

    #[test]
    fn pops_lands_and_bails_from_state_changes() {
        let mut c = Cues::default();
        let mut ev = Vec::new();
        let dt = 1.0 / 60.0;
        c.tick(100, 0.0, 5.0, dt, &mut ev);
        assert_eq!(c.tick(100, 0.0, 5.0, dt, &mut ev), (5.0, 0.0, 0.0));
        c.tick(200, 0.1, 5.0, dt, &mut ev);
        let mut h = 0.1;
        for _ in 0..20 {
            h -= 0.1;
            c.tick(201, h, 5.0, dt, &mut ev);
        }
        c.tick(100, h, 5.0, dt, &mut ev);
        c.tick(300, h, 2.0, dt, &mut ev);
        assert_eq!(ev[0], Event::Pop);
        assert!(matches!(ev[1], Event::Land(s) if s > 0.8));
        assert_eq!(ev[2], Event::Bail);
        assert_eq!(c.tick(401, 0.0, 4.0, dt, &mut ev), (0.0, 4.0, 0.0));
        assert_eq!(c.tick(101, 0.0, 4.0, dt, &mut ev), (0.0, 0.0, 4.0));
    }
}
