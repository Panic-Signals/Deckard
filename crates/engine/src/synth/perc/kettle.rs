//! Percussion's Kettle engine: DECKARD's own design on
//! [`modal_lite`](crate::synth::modal_lite), tuned against Maschine's renders (not a
//! port; docs/synth-perc.md §5).
//!
//! A timpani: a struck membrane whose modes sit near 1, 1.49, 2.04, 2.47, 2.93 and 3.42
//! times the fundamental and ring for over a second; Damp shortens them and pulls the
//! pitch down a little; Puff adds a breathy high tail. Measured in Maschine: 108 Hz at
//! Tune 0.5 on note 60, an octave each way over Tune; the note moves the pitch about an
//! octave per 20 semitones (note 40 plays as Tune 0).

use crate::synth::modal_lite::{Noise, Patch, Source, Voice, expmap, lin, vel_gain};

/// The membrane's modes: (ratio, level (dB), decay share).
const MODES: [(f32, f32, f32); 8] = [
    (1.0, 0.0, 0.45),
    (2.04, -5.0, 0.6),
    (1.49, -8.0, 1.0),
    (3.42, -16.0, 0.45),
    (2.93, -16.0, 0.5),
    (2.47, -17.0, 0.55),
    (4.47, -17.0, 0.35),
    (3.9, -18.0, 0.4),
];

pub struct Kettle {
    voice: Voice,
    /// 0 tune, 1 decay, 2 impact, 3 puff, 4 damp, 5 velocity.
    params: [f32; 8],
}

impl Default for Kettle {
    fn default() -> Kettle {
        Kettle::new()
    }
}

impl Kettle {
    pub fn new() -> Kettle {
        Kettle { voice: Voice::new(44_100.0), params: [0.5, 0.5, 0.6, 0.25, 0.0, 1.0, 0.0, 0.0] }
    }
    pub fn set_sample_rate(&mut self, sr: f32, _block: i32) {
        self.voice.set_sample_rate(sr);
    }
    pub fn reset(&mut self) {
        self.voice.reset();
    }
    pub fn set_param(&mut self, idx: u32, v: f32) {
        if let Some(p) = self.params.get_mut(idx as usize) {
            *p = v;
        }
    }
    #[allow(dead_code)]
    pub fn param(&self, idx: u32) -> f32 {
        self.params.get(idx as usize).copied().unwrap_or(0.0)
    }
    pub fn note_on(&mut self, vel: f32, note: u32) {
        let [tune, decay, impact, puff, damp, velocity, ..] = self.params;
        let k = ((tune - 0.5) * 2.0 + (note as f32 - 60.0) / 20.0).exp2();
        let f0 = 108.0 * k * lin(damp, 1.0, 0.85);
        let tail = expmap(decay, 0.12, 0.6) * lin(damp, 1.0, 0.35);
        let mut x = Patch::default();
        for &(r, db, share) in &MODES {
            x.mode(f0 * r, 10f32.powf(db / 20.0), tail * share, 0.0);
        }
        x.bend = 0.08;
        x.bend_fall = 0.02;
        x.click = Noise {
            level: lin(impact, 0.2, 1.0),
            source: Source::White,
            freq: 1500.0,
            q: 0.7,
            hp: 60.0,
            attack: 0.0,
            decay: 0.006,
            width: 0.0,
        };
        // Puff: a breathy tail around the upper modes.
        x.noise = Noise {
            level: puff * 0.05,
            source: Source::Modal(4),
            freq: f0 * 5.0,
            q: 2.0,
            hp: 200.0,
            attack: 0.02,
            decay: lin(puff, 0.15, 1.2),
            width: 0.0,
        };
        x.gain = 0.3 * vel_gain(vel, velocity) * lin(impact, 0.8, 1.1);
        self.voice.strike(x);
    }
    pub fn choke(&mut self) {
        self.voice.choke();
    }
    pub fn control(&mut self) {}
    pub fn has_output(&self) -> bool {
        self.voice.active()
    }
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.voice.process(l, r);
    }
}
