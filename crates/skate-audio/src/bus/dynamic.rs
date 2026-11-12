//! The dynamic-object manager's two bus graphs (`CSTATEMGR_DynamicObject`, built by `sub_824F46B8`; research b640,
//! b641): `Sub0 → Gai0 → FSS0 → PI20 → PI20 → Pn21 (1 → 6) → Sen0 (→ SFX Master)`. Graph A takes the voices whose
//! template routes code 2048, graph B code 512 (the pushed props' main loops, the sliding start loop); the manager
//! posts their parameters every tick (`world::moveable::Manager`): docs/hails-additions/27-dynamic-props.md#prop-sounds
use crate::dsp::fss::FrequencyShift;
use crate::dsp::gain::Gain;
use crate::dsp::pan::{self, Pan2D};
use crate::dsp::peaking::PeakingIir2;
use crate::dsp::routes::to_six;
use crate::dsp::send::{Mode, Send};
use crate::{BLOCK, MIX_RATE};

/// The routing codes of graphs A and B.
pub const CODES: [u16; 2] = [2048, 512];

/// One peaking EQ's parameters (PI20 p0 centre Hz, p1 linear gain, p2 Q).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Peak {
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
}

/// What the manager posts to one graph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DynamicParams {
    /// Gai0 p0 (linear), FSS0 p0 (Hz), the two PI20s, Pn21 p0 (degrees).
    pub gain: f32,
    pub shift: f32,
    pub peaks: [Peak; 2],
    pub angle: f32,
}

#[derive(Clone, Debug)]
struct Graph {
    /// Sub0: voices add into channel 1 (the mono route).
    input: Box<[[f32; BLOCK]; 6]>,
    gain: Gain,
    fss: FrequencyShift,
    peaks: [PeakingIir2; 2],
    pan: Pan2D,
    send: Send,
}

impl Default for Graph {
    fn default() -> Self {
        Self {
            input: Box::new([[0.0; BLOCK]; 6]),
            gain: Gain::default(),
            fss: FrequencyShift::default(),
            peaks: [PeakingIir2::default(), PeakingIir2::default()],
            pan: Pan2D::new(1),
            send: Send::default(),
        }
    }
}

impl Graph {
    fn render(&mut self, master: &mut [[f32; BLOCK]; 6]) {
        let mut mono = self.input[1];
        self.input[1] = [0.0; BLOCK];
        {
            let mut planes: [&mut [f32]; 1] = [&mut mono[..]];
            self.gain.process(&mut planes);
        }
        self.fss.process(&mut mono[..], MIX_RATE as f32);
        {
            let mut planes: [&mut [f32]; 1] = [&mut mono[..]];
            for p in &mut self.peaks {
                p.process(&mut planes, MIX_RATE as f32);
            }
        }
        let mut six = [[0.0f32; BLOCK]; 6];
        self.pan.process(&[&mono[..]], &mut six);
        let outs = six.each_ref().map(|c| &c[..]);
        self.send.process(&outs, to_six(6), master, Mode::Normal);
    }
}

/// Both graphs; built at the first parameter post (`sub_824F46B8` runs at the manager's init; ours waits for the
/// first pushed prop so maps without one render nothing). Graph B starts silent (its Gai0 p0 = 0.0 at the build).
#[derive(Clone, Debug, Default)]
pub struct DynamicBuses {
    graphs: Option<Box<[Graph; 2]>>,
}

impl DynamicBuses {
    pub fn built(&self) -> bool {
        self.graphs.is_some()
    }

    /// Post the manager's parameters to graph `i` (0 = A, 1 = B), building both graphs on the first post.
    pub fn set(&mut self, i: usize, p: &DynamicParams) {
        let graphs = self.graphs.get_or_insert_with(|| {
            let mut g: Box<[Graph; 2]> = Box::new([Graph::default(), Graph::default()]);
            g[1].gain.target = 0.0;
            g
        });
        let Some(g) = graphs.get_mut(i) else { return };
        g.gain.target = p.gain;
        g.fss.shift_hz = p.shift;
        for (eq, v) in g.peaks.iter_mut().zip(p.peaks) {
            eq.freq = v.freq;
            eq.gain = v.gain;
            eq.q = v.q;
        }
        g.pan.params[pan::ANGLE] = p.angle;
    }

    /// The input of the graph a fixed routing code feeds (None before the build or for other codes).
    pub fn input(&mut self, code: u16) -> Option<&mut [[f32; BLOCK]; 6]> {
        let i = CODES.iter().position(|&c| c == code)?;
        self.graphs.as_mut().map(|g| &mut *g[i].input)
    }

    pub fn render(&mut self, master: &mut [[f32; BLOCK]; 6]) {
        if let Some(graphs) = self.graphs.as_mut() {
            for g in graphs.iter_mut() {
                g.render(master);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_routed_voice_reaches_master_through_graph_a_after_the_build() {
        let mut d = DynamicBuses::default();
        assert!(d.input(2048).is_none(), "no graphs before the first post");
        let p = DynamicParams { gain: 1.0, shift: 0.0, peaks: [Peak { freq: 1000.0, gain: 1.0, q: 1.0 }; 2], angle: 0.0 };
        d.set(0, &p);
        assert!(d.input(512).is_some() && d.input(4096).is_none());
        d.input(2048).unwrap()[1] = [0.5; BLOCK];
        let mut master = [[0.0f32; BLOCK]; 6];
        for _ in 0..4 {
            d.render(&mut master);
            d.input(2048).unwrap()[1] = [0.5; BLOCK];
        }
        assert!(master.iter().flatten().any(|v| v.abs() > 0.01), "graph A renders its input into SFX Master");
    }
}
