//! Scripted input scenarios. Project-owned input data: open-loop XInput
//! pad states per tick, identical for the reference and the candidate.
//!
//! XInput conventions: stick Y is positive up, full deflection ±32767, triggers 0..255.
//! Button bits: A 0x1000, B 0x2000, X 0x4000, Y 0x8000, LB 0x0100, RB 0x0200,
//! L3 0x0040, R3 0x0080, D-pad up/down/left/right 0x1/0x2/0x4/0x8.

pub const TICKS: u32 = 3600;

pub const A: u16 = 0x1000;
pub const B: u16 = 0x2000;
pub const X: u16 = 0x4000;
pub const Y: u16 = 0x8000;
pub const RB: u16 = 0x0200;
pub const L3: u16 = 0x0040;
pub const R3: u16 = 0x0080;

const FULL: f32 = 32767.0;

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Pad {
    pub buttons: u16,
    pub triggers: [u8; 2],
    pub left: [i16; 2],
    pub right: [i16; 2],
}

pub struct Scenario {
    pub name: &'static str,
    pub difficulty: &'static str,
    pub pads: Vec<Pad>,
}

impl Scenario {
    fn new(name: &'static str) -> Self {
        Self { name, difficulty: "easy", pads: vec![Pad::default(); TICKS as usize] }
    }
    fn span(&mut self, from: u32, to: u32, f: impl Fn(&mut Pad)) -> &mut Self {
        for pad in &mut self.pads[from as usize..(to as usize).min(TICKS as usize)] {
            f(pad);
        }
        self
    }
    fn buttons(&mut self, from: u32, to: u32, bits: u16) -> &mut Self {
        self.span(from, to, |p| p.buttons |= bits)
    }
    fn left(&mut self, from: u32, to: u32, v: [i16; 2]) -> &mut Self {
        self.span(from, to, |p| p.left = v)
    }
    fn right(&mut self, from: u32, to: u32, v: [i16; 2]) -> &mut Self {
        self.span(from, to, |p| p.right = v)
    }
    fn triggers(&mut self, from: u32, to: u32, v: [u8; 2]) -> &mut Self {
        self.span(from, to, |p| p.triggers = v)
    }
    /// `count` pushes: A held `hold` ticks, then released for `gap` ticks.
    fn pushes(&mut self, from: u32, count: u32, hold: u32, gap: u32) -> &mut Self {
        for i in 0..count {
            let t = from + i * (hold + gap);
            self.buttons(t, t + hold, A);
        }
        self
    }
    /// Right-stick path through `(ticks, degrees)` points at full deflection, holding
    /// each point for its tick count. Degrees: 0 right, 90 up, 180 left, 270 down.
    fn right_path(&mut self, from: u32, points: &[(u32, f32)]) -> &mut Self {
        let mut t = from;
        for &(ticks, degrees) in points {
            let v = stick(degrees, 1.0);
            self.right(t, t + ticks, v);
            t += ticks;
        }
        self
    }
    /// Crouch (stick down) for `crouch` ticks, then flick through `flick` angles.
    fn flick(&mut self, from: u32, crouch: u32, flick: &[(u32, f32)]) -> &mut Self {
        self.right_path(from, &[(crouch, 270.0)]);
        self.right_path(from + crouch, flick)
    }
}

/// Truncates toward zero, as the searches that found the timings did: one stick unit
/// can change a trajectory.
pub fn stick(degrees: f32, magnitude: f32) -> [i16; 2] {
    let r = degrees.to_radians();
    let axis = |v: f32| {
        let v = v * magnitude * FULL;
        // Snap tiny cos/sin residues (e.g. cos 90°) to zero.
        if v.abs() < 0.5 { 0 } else { v as i16 }
    };
    [axis(r.cos()), axis(r.sin())]
}

/// Three pushes down the spawn ramp, then a steady right-hand orbit (left stick 0.9
/// from tick 235) around (-6, 23): clear of the rails, climb block and halfpipe, and
/// on the floor for the whole run. Found with a search over steering and start tick.
fn orbit(name: &'static str) -> Scenario {
    let mut s = Scenario::new(name);
    s.pushes(12, 3, 18, 22);
    s.left(235, TICKS, stick(0.0, 0.9));
    s
}

/// A move at `t` on the orbit, with steering released for `free` ticks so the left
/// stick is not read as an air spin.
fn on_orbit(s: &mut Scenario, t: u32, free: u32) {
    s.left(t, t + free, [0, 0]);
}

/// The corpus (the core moves plus a few extra ones). Timings come from searches
/// with the candidate; `sim-oracle states` shows the
/// physical states each scenario reaches.
pub fn all() -> Vec<Scenario> {
    let mut list = Vec::new();

    list.push(Scenario::new("00_idle"));

    // input/manual_replay.rs (the plan's synthetic sequence; +22000 is a nose manual
    // in the default stance), then brake so the skater stays on the floor.
    let mut s = Scenario::new("01_manual_replay");
    s.buttons(12, 40, A).right(45, 195, [0, 22000]).buttons(330, 700, B);
    list.push(s);

    let mut s = orbit("02_push_top_speed");
    s.pushes(132, 87, 18, 22);
    list.push(s);

    for (name, flick) in [
        ("03_ollie", &[(4, 90.0)][..]),
        ("04_kickflip", &[(4, 135.0)][..]),
        ("05_heelflip", &[(4, 45.0)][..]),
    ] {
        let mut s = orbit(name);
        for k in 0..22 {
            let t = 300 + k * 150;
            on_orbit(&mut s, t, 80);
            s.flick(t, 12, flick);
        }
        list.push(s);
    }

    // docs/360-FLIP.md: left/slightly down, down-left, down/slightly right, up-right flick.
    let mut s = orbit("06_360flip");
    for k in 0..22 {
        let t = 300 + k * 150;
        on_orbit(&mut s, t, 80);
        s.right_path(t, &[(6, 200.0), (4, 225.0), (4, 280.0), (4, 45.0)]);
    }
    list.push(s);

    for (name, y) in [("07_manual", -22000), ("08_nose_manual", 22000)] {
        let mut s = orbit(name);
        for k in 0..11 {
            let t = 300 + k * 300;
            s.right(t, t + 180, [0, y]);
        }
        list.push(s);
    }

    // Left stick diagonally down-sideways starts the stock powerslide (searched).
    let mut s = orbit("09_powerslide");
    for k in 0..11 {
        let t = 300 + k * 300;
        s.left(t, t + 60, stick(if k % 2 == 0 { 315.0 } else { 225.0 }, 1.0));
        s.pushes(t + 120, 2, 18, 22);
    }
    list.push(s);

    let mut s = orbit("10_air_spin");
    for k in 0..16 {
        let t = 300 + k * 200;
        on_orbit(&mut s, t, 90);
        s.flick(t, 12, &[(4, 90.0)]);
        s.left(t + 16, t + 60, stick(if k % 2 == 0 { 0.0 } else { 180.0 }, 1.0));
        s.pushes(t + 120, 1, 18, 22);
    }
    list.push(s);

    // Figure eight: one circle each way (about 240 ticks per circle at orbit speed).
    let mut s = orbit("11_carve");
    for k in 0..14 {
        let t = 235 + k * 240;
        s.left(t, t + 240, stick(if k % 2 == 0 { 0.0 } else { 180.0 }, 0.9));
    }
    list.push(s);

    // tests/wipeout_playback.rs: L3 + R3 with both triggers is the bail chord.
    let mut s = orbit("12_bail");
    for k in 0..8 {
        let t = 300 + k * 400;
        s.buttons(t, t + 30, L3 | R3).triggers(t, t + 30, [255; 2]);
        s.pushes(t + 250, 3, 18, 22);
    }
    list.push(s);

    // tests/offboard_*: Y toggles the board, X jumps off-board, left stick walks.
    let mut s = Scenario::new("13_offboard_walk_remount");
    for k in 0..6 {
        let t = 60 + k * 580;
        s.buttons(t, t + 1, Y)
            .left(t + 40, t + 260, stick(90.0 + 60.0 * k as f32, 1.0))
            .buttons(t + 300, t + 301, X)
            .buttons(t + 420, t + 421, Y);
    }
    list.push(s);

    let mut s = orbit("14_hippy_jump");
    for k in 0..16 {
        let t = 300 + k * 200;
        s.buttons(t, t + 40, A | X);
    }
    list.push(s);

    // Grabs (left trigger in the air) and braking on the orbit.
    let mut s = orbit("15_grabs_and_brake");
    for k in 0..8 {
        let t = 300 + k * 400;
        on_orbit(&mut s, t, 90);
        s.flick(t, 12, &[(4, 90.0)])
            .triggers(t + 18, t + 60, [255, 0])
            .buttons(t + 200, t + 260, B)
            .pushes(t + 280, 2, 18, 22);
    }
    list.push(s);

    // Low rail (x = -7, z 7..15): drift right on the ramp, U-turn, ollie at 348.
    // Reaches GrindFiftyFifty then GrindFiveO (searched).
    let mut s = Scenario::new("16_low_rail_5050");
    s.pushes(12, 3, 18, 22)
        .left(140, 180, stick(180.0, 0.7))
        .left(215, 340, stick(0.0, 1.0))
        .flick(348, 12, &[(4, 90.0)])
        .buttons(700, 1100, B);
    list.push(s);

    // Crossing the low rail on a full-lock curve: GrindBoardslide (searched).
    let mut s = Scenario::new("17_low_rail_boardslide");
    s.pushes(12, 3, 18, 22)
        .left(190, 340, stick(0.0, 1.0))
        .flick(332, 12, &[(4, 90.0)])
        .buttons(700, 1100, B);
    list.push(s);

    // The same curve held longer: GrindTipslide (searched).
    let mut s = Scenario::new("18_low_rail_tipslide");
    s.pushes(12, 3, 18, 22)
        .left(200, 400, stick(0.0, 1.0))
        .flick(324, 12, &[(4, 90.0)])
        .buttons(700, 1100, B);
    list.push(s);

    // Halfpipe: drift left, left U-turn into the channel, turn up the x = 16.5 wall and
    // hold RB (grab world) at the lip: HandPlant (searched).
    let mut s = Scenario::new("19_halfpipe_handplant");
    s.pushes(12, 3, 18, 22)
        .left(140, 180, stick(0.0, 0.7))
        .left(200, 325, stick(180.0, 1.0))
        .left(420, 458, stick(0.0, 1.0))
        .buttons(478, 568, RB)
        .buttons(900, 1300, B);
    list.push(s);

    list
}

pub fn write_input(s: &Scenario) -> String {
    let mut out = format!("# difficulty={}\n# scenario={}\n", s.difficulty, s.name);
    for p in &s.pads {
        out.push_str(&format!(
            "{} {} {} {} {} {} {}\n",
            p.buttons, p.triggers[0], p.triggers[1], p.left[0], p.left[1], p.right[0], p.right[1]
        ));
    }
    out
}
