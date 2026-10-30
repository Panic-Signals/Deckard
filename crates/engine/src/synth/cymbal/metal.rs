//! The Cymbal's Crash and Ride: DECKARD's own designs on
//! [`modal_lite`](crate::synth::modal_lite), tuned against Maschine's renders (not
//! ports; docs/synth-cymbal.md §5).
//!
//! A struck plate: dozens of inharmonic modes scattered over the cymbal's band (the same
//! every strike), a noise wash and a stick click. Measured in Maschine at the defaults:
//! the Crash is bright (partials 4.7–7.8 kHz) and very wide, −60 dB at about 1.2 s.
//!
//! The Ride (retuned 2026-10-27 against Maschine's renders at the defaults and each
//! knob's extremes): a plate from 250 Hz up, loudest at 1.6–2.3 kHz (`ride_shape`), whose
//! wash swells in over a few milliseconds (Tail slows it), −60 dB at about 3.6 s. Edge
//! toward 0 shortens it to about 1.2 s and brings the lows up; Tune and the note move the
//! cluster and highs, not the lows; velocity is steep at soft hits.

use crate::synth::modal_lite::{Noise, Patch, Source, Voice, expmap, lin, metal, vel_gain};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Crash,
    Ride,
}

pub(super) struct Metal {
    kind: Kind,
    voice: Voice,
    /// Crash: 0 tune, 1 decay, 2 density, 3 tone, 4 impact, 5 width, 6 mode, 7 velocity.
    /// Ride: 0 tune, 1 decay, 2 edge, 3 bell, 4 impact, 5 character, 6 tail, 7 width,
    /// 8 velocity.
    p: [f32; 9],
}

impl Metal {
    pub(super) fn new(kind: Kind) -> Metal {
        let p = match kind {
            Kind::Crash => [0.3, 0.7, 1.0, 0.5, 0.7, 0.5, 0.0, 1.0, 0.0],
            Kind::Ride => [0.3, 0.7, 0.5, 0.0, 0.7, 0.2, 1.0, 0.5, 1.0],
        };
        Metal { kind, voice: Voice::new(44_100.0), p }
    }
    pub(super) fn on_set_sample_rate(&mut self, sr: f32, _block: i32) {
        self.voice.set_sample_rate(sr);
    }
    pub(super) fn on_reset(&mut self) {
        self.voice.reset();
    }
    pub(super) fn on_choke(&mut self) {
        self.voice.choke();
    }
    pub(super) fn set_parameter(&mut self, i: u32, v: f32) {
        if let Some(x) = self.p.get_mut(i as usize) {
            *x = v;
        }
    }
    pub(super) fn set_note_on(&mut self, v: f32, key: u32) {
        let x = match self.kind {
            Kind::Crash => self.crash(v, key),
            Kind::Ride => self.ride(v, key),
        };
        self.voice.strike(x);
    }
    pub(super) fn process_control_rate(&mut self) {}
    pub(super) fn process_modulation_rate(&mut self) {}
    pub(super) fn process_audio(&mut self, l: &mut [f32], r: &mut [f32]) -> bool {
        let on = self.voice.active();
        self.voice.process(l, r);
        on
    }
    pub(super) fn has_output_signal(&self) -> bool {
        self.voice.active()
    }

    fn crash(&self, vel: f32, key: u32) -> Patch {
        let [tune, decay, density, tone, impact, width, mode, velocity, _] = self.p;
        let k = ((tune - 0.3) * 1.5 + (key as f32 - 60.0) / 24.0).exp2();
        let tail = expmap(decay, 0.07, 0.34);
        let w = lin(width, 0.9, 1.0);
        let mut x = Patch::default();
        // Metallic rings longest with the most modes; Soft is mostly wash.
        let (n, ring, wash) = match mode as u32 {
            0 => (56, 1.0, 0.6),
            1 => (44, 0.75, 1.0),
            _ => (30, 0.5, 1.6),
        };
        let n = (n as f32 * lin(density, 0.4, 1.0)) as usize;
        let tilt = lin(tone, -4.0, 3.0);
        metal(&mut x, 11, n, 2500.0 * k, 13_000.0 * k, tilt, (4500.0 * k, 8000.0 * k), 6.0, tail * ring, 0.3, w);
        x.noise = Noise {
            level: 0.12 * wash,
            source: Source::Modal(20),
            freq: 6000.0 * k * lin(tone, 0.7, 1.3),
            q: 0.5,
            hp: 1800.0,
            attack: 0.002,
            decay: tail * 0.8,
            width: w,
        };
        x.click = Noise {
            level: lin(impact, 0.1, 0.6),
            source: Source::White,
            freq: 7000.0,
            q: 0.6,
            hp: 1500.0,
            attack: 0.0,
            decay: 0.003,
            width: 0.0,
        };
        x.scatter = true;
        x.gain = 0.045 * vel_gain(vel, velocity) * lin(impact, 0.7, 1.15);
        x
    }

    fn ride(&self, vel: f32, key: u32) -> Patch {
        let [tune, decay, edge, bell, impact, character, tail_p, width, velocity] = self.p;
        // Tune and the note are one control (note 48 plays as Tune 0), the pitch rising
        // faster toward the top: measured, −0.4 octave at Tune 0, +1.5 at Tune 1.
        let t = tune - 0.3 + (key as f32 - 60.0) * 0.025;
        let k = (1.54 * t + 0.8 * t * t).exp2();
        // Decay 0.7 (the default) rings about 3.6 s to −60 dB, 0 about 0.8 s, 1 about 4.2 s;
        // hit at the edge (Edge 0) it dies in about a third of that.
        let mut tail = if decay < 0.7 { expmap(decay / 0.7, 0.12, 0.55) } else { lin((decay - 0.7) / 0.3, 0.55, 0.64) };
        tail *= if edge < 0.5 { expmap(edge / 0.5, 0.34, 1.0) } else { lin((edge - 0.5) / 0.5, 1.0, 1.1) };
        let w = lin(width, 0.6, 1.3) * lin(character, 0.7, 1.8);
        let low = (0.5 - edge) * 20.0 - 6.0 * character;
        let mut x = Patch::default();
        // The plate: modes from 250 Hz up, loudest at 1.6–2.3 kHz as measured in Maschine,
        // the highs dying first (faster still toward the edge).
        let mut s = 23u32.wrapping_mul(2_654_435_761) | 1;
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s as f32 / u32::MAX as f32
        };
        let n = 56;
        let (lo, hi) = (250.0, 16_000.0 * k.max(1.0));
        for i in 0..n {
            let f = lo * ((i as f32 + rnd()) / n as f32 * (hi / lo).log2()).exp2();
            let o = (f / (1800.0 * k)).log2();
            // The lows stay put as Tune moves the rest; the highs fall no lower than untuned.
            let at = if f < 1200.0 {
                ride_shape(f)
            } else if f > 4000.0 {
                ride_shape(f / k).max(ride_shape(f))
            } else {
                ride_shape(f / k)
            };
            let db = at - 6.0 * rnd() + if f < 1200.0 { low } else { 0.0 };
            let short = if o > 0.0 { lin(edge, 0.4, 0.1) * o } else { -0.5 * o };
            let d = tail * (-short).exp2() * (0.7 + 0.6 * rnd());
            let pan = (rnd() * 2.0 - 1.0) * w;
            x.mode(f, 10f32.powf(db / 20.0), d, pan.clamp(-1.0, 1.0));
        }
        // The bell: a cluster of strong partials at 1.6–4.5 kHz.
        for (i, &(r, db)) in [(1.0, 0.0), (1.47, -2.0), (2.0, -3.0), (2.76, -5.0)].iter().enumerate() {
            let pan = if i % 2 == 0 { 0.3 } else { -0.3 } * w;
            x.mode(1650.0 * k * r, bell * 1.4 * 10f32.powf(db / 20.0), tail * 1.2, pan);
        }
        // The wash builds over the first tens of milliseconds (more slowly with Tail).
        x.swell = lin(tail_p, 0.002, 0.012);
        x.noise = Noise {
            level: 0.15,
            source: Source::Modal(20),
            freq: 2500.0 * k,
            q: 0.4,
            hp: 300.0,
            attack: lin(tail_p, 0.002, 0.012),
            decay: tail * 0.5,
            width: w.min(1.0),
        };
        x.click = Noise {
            level: lin(impact, 0.8, 3.0),
            source: Source::White,
            freq: 2000.0,
            q: 0.5,
            hp: 400.0,
            attack: 0.0,
            decay: 0.008,
            width: 0.0,
        };
        x.scatter = true;
        // A shorter ring is struck louder, as in Maschine (Decay 0 loses only about 2 dB).
        let short = (0.55 / tail).powf(0.35);
        // Velocity: steep at soft hits, gentle at hard ones (30 is about 37 dB below 100).
        x.gain = 0.04
            * short
            * vel_gain(vel, velocity).powf(1.0 + 0.9 * (1.0 - vel.clamp(0.0, 1.0)))
            * 10f32.powf(((edge - 0.5) * 6.0 + bell * 2.5 + character * 6.0) / 20.0);
        x
    }
}

/// The Ride's plate level (dB) at `f` Hz (Tune 0.3, note 60), from Maschine's renders.
fn ride_shape(f: f32) -> f32 {
    const PTS: [(f32, f32); 13] = [
        (200.0, -39.0),
        (283.0, -21.0),
        (400.0, -12.0),
        (566.0, -15.0),
        (800.0, -13.0),
        (1130.0, -15.0),
        (1600.0, 0.0),
        (2260.0, -3.0),
        (3200.0, -8.0),
        (4530.0, -12.0),
        (6400.0, -20.0),
        (9050.0, -23.0),
        (12_800.0, -32.0),
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
    a.1 + (b.1 - a.1) * (x - a.0.log2()) / (b.0.log2() - a.0.log2())
}
