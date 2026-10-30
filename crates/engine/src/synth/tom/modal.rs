//! Tom's Floor and High engines: DECKARD's own designs on
//! [`modal_lite`](crate::synth::modal_lite), tuned against Maschine's renders (not ports;
//! docs/synth-tom.md §5).
//!
//! A tensioned skin: its modes start bent up and glide down (Tension sets how far), Tune
//! moves the skin an octave, and fixed shell modes ring under it. Floor adds a soft
//! stereo "air" (pink noise) and its Flex and Skin controls; High is mono. Measured in
//! Maschine: Floor settles near 67–76 Hz at Tune 0 with shell modes at 105/117 Hz that
//! don't follow Tune; High falls from about 199 to 124 Hz over half a second.

use crate::synth::modal_lite::{Noise, Patch, Source, Voice, expmap, lin, vel_gain};

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Floor,
    High,
}

pub struct Core {
    kind: Kind,
    voice: Voice,
    /// Floor: 0 flex, 1 tune, 2 tension, 3 impact, 4 decay, 5 air, 6 skin, 7 velocity;
    /// High: 0 tune, 1 decay, 2 tension, 3 impact, 4 velocity.
    p: [f32; 8],
}

impl Core {
    pub fn high() -> Core {
        Core { kind: Kind::High, voice: Voice::new(48_000.0), p: [0.0, 0.5, 1.0, 0.8, 1.0, 0.0, 0.0, 0.0] }
    }
    pub fn floor() -> Core {
        Core { kind: Kind::Floor, voice: Voice::new(48_000.0), p: [0.3, 0.0, 1.0, 0.8, 0.5, 0.5, 0.0, 1.0] }
    }
    pub fn set_sample_rate(&mut self, sr: f32) {
        self.voice.set_sample_rate(sr);
    }
    pub fn reset(&mut self) {
        self.voice.reset();
    }
    pub fn set_parameter(&mut self, idx: u32, v: f32) {
        if let Some(x) = self.p.get_mut(idx as usize) {
            *x = v.clamp(-1.0, 1.0);
        }
    }
    pub fn note_on(&mut self, vel: f32, note: u32) {
        let x = match self.kind {
            Kind::Floor => self.floor_patch(vel, note),
            Kind::High => self.high_patch(vel, note),
        };
        self.voice.strike(x);
    }
    pub fn note_off(&mut self, _note: u32) {}
    pub fn choke(&mut self) {
        self.voice.choke();
    }
    pub fn control_rate(&mut self) {}
    pub fn output(&self) -> bool {
        self.voice.active()
    }
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.voice.process(l, r);
    }

    fn floor_patch(&self, vel: f32, note: u32) -> Patch {
        let [flex, tune, tension, impact, decay, air, skin, velocity] = self.p;
        let k = (tune + (note as f32 - 60.0) / 12.0).exp2();
        let tail = if decay < 0.5 { lin(decay * 2.0, 0.05, 0.125) } else { lin(decay * 2.0 - 1.0, 0.125, 0.265) };
        let mut x = Patch::default();
        let f0 = 70.0 * k;
        x.mode(f0, 1.0, tail, 0.0);
        // Skin overtones (Flex stretches them), and the shell, which stays put.
        let stretch = lin(flex, 0.92, 1.12);
        for &(r, db) in &[(3.4, -16.0), (3.6, -17.0), (3.83, -18.0), (1.93, -20.0)] {
            x.mode(f0 * r * stretch, 10f32.powf(db / 20.0), tail * 0.4, 0.1);
        }
        x.mode(105.0, 0.32, tail * 0.7, -0.15);
        x.mode(117.0, 0.2, tail * 0.6, 0.15);
        x.bend = 0.55 * tension;
        x.bend_fall = 0.05;
        // Skin: a damped skin knocks shorter and brighter.
        x.click = Noise {
            level: lin(impact, 0.5, 2.0) * lin(skin, 0.6, 1.2),
            source: Source::White,
            freq: lin(skin, 900.0, 2500.0),
            q: 0.8,
            hp: 60.0,
            attack: 0.0,
            decay: 0.006,
            width: 0.0,
        };
        x.noise =
            Noise { level: air * 0.12, source: Source::Pink, freq: 400.0, q: 0.5, hp: 60.0, attack: 0.004, decay: tail * 0.8, width: 1.0 };
        x.gain = 0.27 * vel_gain(vel, velocity) * lin(impact, 0.8, 1.1);
        x
    }

    fn high_patch(&self, vel: f32, note: u32) -> Patch {
        let [tune, decay, tension, impact, velocity, ..] = self.p;
        let k = (tune * 0.95 + (note as f32 - 60.0) / 12.0).exp2();
        let tail = if decay < 0.5 { lin(decay * 2.0, 0.05, 0.08) } else { lin(decay * 2.0 - 1.0, 0.08, 0.11) };
        let mut x = Patch::default();
        let f0 = 118.0 * k;
        x.mode(f0, 1.0, tail * 1.2, 0.0);
        for &(r, db, sh) in &[(1.47, -11.0, 0.8), (1.66, -14.0, 0.7), (2.13, -24.0, 0.5), (2.45, -24.0, 0.45), (2.67, -26.0, 0.4)] {
            x.mode(f0 * r, 10f32.powf(db / 20.0), tail * sh, 0.0);
        }
        // A long glide down: the strike starts high and keeps falling.
        x.bend = lin(tension, 0.3, 0.78);
        x.bend_fall = expmap(tension, 0.08, 0.12);
        x.click = Noise {
            level: lin(impact, 0.2, 0.8),
            source: Source::White,
            freq: 2000.0,
            q: 0.8,
            hp: 80.0,
            attack: 0.0,
            decay: 0.005,
            width: 0.0,
        };
        x.gain = 0.4 * vel_gain(vel, velocity) * lin(impact, 0.8, 1.1);
        x
    }
}
