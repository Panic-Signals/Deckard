//! The Hi-hat's Hybrid engine: DECKARD's own design on
//! [`modal_lite`](crate::synth::modal_lite), tuned against Maschine's renders (not a
//! port; docs/synth-hihat.md §6).
//!
//! Small cymbals: inharmonic modes from 400 Hz to 20 kHz shaped as Maschine's renders
//! measure (`shape`), the high ones dying first; Metallic brings more of them up around
//! 6 kHz; a broad noise "rattle"; and the stick's bright strike over the first 10 ms.
//! Tune moves the metal as the note does (Tune 0 sounds as note 48, 1 as note 72), Color
//! darkens it below its default, Character is mostly level, and a soft hit dies sooner.
//! Measured in Maschine at the defaults: mono, peak energy 5–8 kHz, −60 dB at about 0.8 s.

use crate::synth::modal_lite::{Noise, Patch, Source, Voice, expmap, lin, vel_gain};

pub struct Hybrid {
    voice: Voice,
    /// tune, decay, color, impact, metallic, bend, rattle, character, velocity scaling.
    pub p: [f32; 9],
    l: [f32; 16],
    r: [f32; 16],
}

impl Hybrid {
    pub fn new(sr: f32) -> Hybrid {
        let mut voice = Voice::new(sr);
        voice.set_sample_rate(sr);
        Hybrid { voice, p: [0.5, 0.65, 0.715, 0.8, 0.0, 0.5, 0.5, 0.25, 1.0], l: [0.0; 16], r: [0.0; 16] }
    }

    pub fn note_on(&mut self, vel: f32, note: u32) {
        let [tune, decay, color, impact, metallic, bend, rattle, character, velocity] = self.p;
        // Tune spans a note an octave down to one an octave up (Tune 0 plays as note 48 and 1 as
        // note 72 there), the metal moving half an octave per octave of note.
        let k = ((tune - 0.5) + (note as f32 - 60.0) / 24.0).exp2();
        let v = vel.clamp(0.0, 1.0);
        let tail = expmap(decay, 0.04, 0.45) * k.powf(0.7) * (0.15 + 0.85 * v) / 0.82;
        // Color darkens the metal below its default and changes little above it.
        let dark = 1.0 - (color / 0.715).min(1.0);
        let lift = 1.0 - 0.18 * dark;
        let mut x = Patch::default();
        // Small cymbals ring from about 500 Hz to the top of hearing, loudest at 5–8 kHz.
        let n = (40.0 + 24.0 * metallic) as usize;
        let mut s = 31u32.wrapping_mul(2_654_435_761) | 1;
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s as f32 / u32::MAX as f32
        };
        let (lo, hi) = (400.0, 20_000.0 * k);
        for i in 0..n {
            let f = lo * ((i as f32 + rnd()) / n as f32 * (hi / lo).log2()).exp2();
            let over = (f / (5000.0 * k)).log2().max(0.0);
            let db = shape(f / (k * lift)) - 6.0 * rnd() - metallic * 4.0 * (f / (6000.0 * k)).log2().abs() - 12.0 * dark * over;
            let o = (f / (7000.0 * k)).log2();
            let d = tail * (if o < 0.0 { 0.25 * o } else { -0.3 * o }).exp2() * (0.7 + 0.6 * rnd());
            x.mode(f, 10f32.powf(db / 20.0), d, 0.0);
        }
        x.scatter = true;
        x.bend = (bend - 0.5) * 1.2;
        x.bend_fall = 0.04;
        x.noise = Noise {
            level: lin(rattle, 0.0, 0.5) * lin(metallic, 1.0, 0.5),
            source: Source::Modal(20),
            freq: 5000.0 * k * lift,
            q: 0.3,
            hp: 400.0,
            attack: 0.001,
            decay: tail * 0.7,
            width: 0.0,
        };
        // The stick's "ting": a bright, broad burst over the first few milliseconds.
        x.click = Noise {
            level: lin(impact, 25.0, 60.0),
            source: Source::White,
            freq: 10_000.0 * lift.sqrt(),
            q: 0.3,
            hp: 300.0,
            attack: 0.0,
            decay: 0.01,
            width: 0.0,
        };
        x.gain =
            0.0075 * vel_gain(vel, velocity).sqrt() * k.sqrt() * (1.0 - 0.45 * dark) * lin(impact, 0.8, 1.1) * expmap(character, 0.8, 2.6);
        self.voice.strike(x);
    }

    pub fn choke(&mut self) {
        self.voice.choke();
    }

    pub fn active(&self) -> bool {
        self.voice.active()
    }

    /// Render into `out` (mono, ≤ 16 frames at a time from the wrapper).
    pub fn process(&mut self, out: &mut [f32]) {
        for chunk in out.chunks_mut(16) {
            let n = chunk.len();
            self.voice.process(&mut self.l[..n], &mut self.r[..n]);
            chunk.copy_from_slice(&self.l[..n]);
        }
    }
}

/// The metal's level (dB) at `f` Hz (at Tune 0.5, note 60, Color 0.715): measured in
/// Maschine's renders, a broad peak at 5–8 kHz, about −12 dB around 1 kHz and −17 dB at
/// 20 kHz.
fn shape(f: f32) -> f32 {
    const PTS: [(f32, f32); 9] = [
        (500.0, -14.0),
        (1000.0, -12.0),
        (2000.0, -8.0),
        (3200.0, -4.0),
        (5000.0, 0.0),
        (8000.0, 0.0),
        (12_800.0, -5.0),
        (16_000.0, -10.0),
        (20_000.0, -12.0),
    ];
    let x = f.max(1.0).log2();
    let i = PTS.iter().position(|p| p.0.log2() > x).unwrap_or(PTS.len());
    if i == 0 {
        return PTS[0].1;
    }
    if i == PTS.len() {
        return PTS[i - 1].1 - 20.0 * (x - PTS[i - 1].0.log2());
    }
    let (a, b) = (PTS[i - 1], PTS[i]);
    let t = (x - a.0.log2()) / (b.0.log2() - a.0.log2());
    a.1 + (b.1 - a.1) * t
}
