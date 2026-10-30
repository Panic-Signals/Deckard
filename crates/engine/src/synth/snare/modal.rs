//! The Snare's modal engines (Airy, Vintage, Chrome, Iron, Breaker): DECKARD's own
//! designs on [`modal_lite`](crate::synth::modal_lite), tuned against Maschine's renders
//! (not ports; docs/synth-snare.md §5).
//!
//! A struck skin: a bank of modes over the tuned fundamental (Skin Tune moves the upper
//! ones against it), a click from Impact, and "Spectra": a few resonances at a pitch of
//! their own (Spectra Tune, not the note) with a little modal noise, for the wires.
//! Measured in Maschine on Airy: Tune spans an octave each way (91…185…366 Hz), Decay
//! stretches −60 dB from 195 to 575 ms, Spectra adds resonances near 132–146 Hz.

use super::Core;
use crate::synth::modal_lite::{Noise, Patch, Source, Voice, expmap, lin, vel_gain};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Airy,
    Vintage,
    Chrome,
    Iron,
    Breaker,
}

/// An engine's sound at the defaults.
struct Profile {
    /// The fundamental at Tune 0.5 on note 60 (Hz).
    f0: f32,
    /// Skin modes: (ratio, level (dB), decay share, moved by Skin Tune).
    modes: &'static [(f32, f32, f32, bool)],
    /// Tail time constant at Decay 0, 0.5 and 1 (s).
    decay: (f32, f32, f32),
    /// Spectra resonances at Spectra Tune 0.5 (Hz, dB).
    spectra: &'static [(f32, f32)],
    /// Modal noise used for the wires, and its band (Hz).
    wires: (usize, f32),
    click: f32,
    gain: f32,
    width: f32,
}

const AIRY: Profile = Profile {
    f0: 185.0,
    modes: &[
        (1.0, 0.0, 1.0, false),
        (0.886, -6.0, 0.9, true),
        (1.52, -12.0, 0.6, true),
        (1.124, -18.0, 0.7, true),
        (1.757, -24.0, 0.6, true),
    ],
    decay: (0.035, 0.075, 0.105),
    spectra: &[(132.0, -15.0), (146.0, -20.0)],
    wires: (11, 3500.0),
    click: 0.5,
    gain: 0.5,
    width: 0.012,
};

const VINTAGE: Profile = Profile {
    f0: 231.0,
    modes: &[
        (1.0, 0.0, 1.0, false),
        (1.368, -4.0, 0.8, true),
        (2.537, -12.0, 0.7, true),
        (1.9, -16.0, 0.7, true),
        (5.34, -18.0, 0.7, true),
    ],
    decay: (0.022, 0.045, 0.065),
    spectra: &[(208.0, -34.0), (252.0, -35.0)],
    wires: (8, 4000.0),
    click: 0.9,
    gain: 0.45,
    width: 0.012,
};

const CHROME: Profile = Profile {
    f0: 270.0,
    modes: &[
        (1.0, 0.0, 1.0, false),
        (0.956, -4.0, 0.9, true),
        (1.03, -4.0, 0.9, true),
        (2.92, -4.0, 0.7, true),
        (1.456, -4.0, 0.7, true),
        (3.94, -7.0, 0.7, true),
        (4.11, -11.0, 0.7, true),
        (4.39, -12.0, 0.7, true),
    ],
    decay: (0.03, 0.065, 0.095),
    spectra: &[(230.0, -24.0)],
    wires: (20, 5000.0),
    click: 1.0,
    gain: 0.22,
    width: 0.01,
};

const IRON: Profile = Profile {
    f0: 231.0,
    modes: &[
        (1.0, 0.0, 1.0, false),
        (1.065, -4.0, 0.9, true),
        (0.775, -4.0, 0.9, true),
        (2.55, -5.0, 0.7, true),
        (2.47, -7.0, 0.7, true),
        (1.558, -9.0, 0.7, true),
    ],
    decay: (0.022, 0.048, 0.07),
    spectra: &[(180.0, -24.0)],
    wires: (7, 4500.0),
    click: 1.0,
    gain: 0.24,
    width: 0.0,
};

const BREAKER: Profile = Profile {
    f0: 272.0,
    modes: &[
        (1.0, 0.0, 1.0, false),
        (1.423, -4.0, 0.8, true),
        (0.915, -4.0, 0.9, true),
        (2.35, -10.0, 0.7, true),
        (2.97, -14.0, 0.7, true),
        (2.31, -14.0, 0.7, true),
        (2.9, -14.0, 0.7, true),
    ],
    decay: (0.03, 0.065, 0.09),
    spectra: &[(240.0, -24.0)],
    wires: (21, 3000.0),
    click: 1.2,
    gain: 0.15,
    width: 0.0,
};

/// One modal Snare engine.
pub(crate) struct ModalSnare {
    kind: Kind,
    voice: Voice,
    /// tune, decay, skin tune, impact, body, spectra mode, spectra tune, spectra
    /// amount, spectra decay, velocity (by meaning; the core numbering is mapped in
    /// `set_parameter`).
    p: [f32; 10],
}

impl ModalSnare {
    pub(crate) fn new(kind: Kind, sr: f32) -> ModalSnare {
        ModalSnare { kind, voice: Voice::new(sr), p: [0.5, 0.5, 0.5, 0.75, 0.75, 0.0, 0.5, 0.75, 0.4, 1.0] }
    }

    fn profile(&self) -> &'static Profile {
        match self.kind {
            Kind::Airy => &AIRY,
            Kind::Vintage => &VINTAGE,
            Kind::Chrome => &CHROME,
            Kind::Iron => &IRON,
            Kind::Breaker => &BREAKER,
        }
    }

    fn patch(&self, vel: f32, note: u32) -> Patch {
        let pr = self.profile();
        let [tune, decay, skin, impact, body, smode, stune, samount, sdecay, velocity] = self.p;
        let f0 = pr.f0 * ((tune - 0.5) * 2.0 + (note as f32 - 60.0) / 12.0).exp2();
        let skin_k = ((skin - 0.5) * 0.6).exp2();
        let tail = if decay < 0.5 { lin(decay * 2.0, pr.decay.0, pr.decay.1) } else { lin(decay * 2.0 - 1.0, pr.decay.1, pr.decay.2) };
        let mut x = Patch::default();
        // Body (Iron) brings the upper modes up.
        let upper = if self.kind == Kind::Iron { lin(body, 0.5, 1.3) } else { 1.0 };
        for (i, &(ratio, db, share, moves)) in pr.modes.iter().enumerate() {
            let r = if moves { ratio * skin_k } else { ratio };
            let lvl = 10f32.powf(db / 20.0) * if i > 0 { upper } else { 1.0 };
            let pan = if i % 2 == 0 { pr.width } else { -pr.width };
            x.mode(f0 * r, lvl, tail * share, pan);
        }
        // Spectra: resonances at their own pitch, and the wires' noise.
        let sk = ((stune - 0.5) * 2.0).exp2() * if smode >= 0.5 { 1.3 } else { 1.0 };
        let sd = expmap(sdecay, 0.03, 0.4);
        for &(f, db) in pr.spectra {
            x.mode(f * sk, 10f32.powf(db / 20.0) * samount * 0.8, sd, 0.0);
        }
        x.noise = Noise {
            level: samount * 0.05,
            source: Source::Modal(pr.wires.0),
            freq: pr.wires.1 * sk,
            q: 0.8,
            hp: 300.0,
            attack: 0.0005,
            decay: sd * 0.5,
            width: pr.width * 10.0,
        };
        x.click = Noise {
            level: pr.click * lin(impact, 0.3, 1.4),
            source: Source::White,
            freq: 2500.0,
            q: 0.7,
            hp: 100.0,
            attack: 0.0,
            decay: 0.003,
            width: 0.0,
        };
        x.bend = 0.12 * impact;
        x.bend_fall = 0.01;
        x.gain = pr.gain * vel_gain(vel, velocity) * lin(impact, 0.7, 1.15);
        x
    }
}

impl Core for ModalSnare {
    /// The core's numbering (`core_index` in snare.rs): Airy, Vintage, Chrome: 0 skin,
    /// 1 tune, 2 decay, 3 impact, 4 spectra amount, 5 spectra decay, 6 spectra tune,
    /// 7 spectra mode, 8 velocity; Iron: 4 spectra decay, 5 spectra amount, 6 body;
    /// Breaker: 0 tune, 1 decay, 2 skin, 4 spectra tune, 5 spectra decay, 6 amount.
    fn set_parameter(&mut self, idx: u32, v: f32) {
        // Meanings: 0 tune, 1 decay, 2 skin, 3 impact, 4 body, 5 spectra mode, 6 spectra
        // tune, 7 spectra amount, 8 spectra decay, 9 velocity.
        let slot = match (self.kind, idx) {
            (Kind::Breaker, 0) => 0,
            (Kind::Breaker, 1) => 1,
            (Kind::Breaker, 2) => 2,
            (Kind::Breaker, 4) => 6,
            (Kind::Breaker, 5) => 8,
            (Kind::Breaker, 6) => 7,
            (Kind::Iron, 4) => 8,
            (Kind::Iron, 5) => 7,
            (Kind::Iron, 6) => 4,
            (_, 0) => 2,
            (_, 1) => 0,
            (_, 2) => 1,
            (_, 3) => 3,
            (_, 4) => 7,
            (_, 5) => 8,
            (_, 6) => 6,
            (_, 7) => 5,
            (_, 8) => 9,
            _ => return,
        };
        self.p[slot] = v;
    }
    fn note_on(&mut self, velocity: f32, note: u32) {
        let x = self.patch(velocity, note);
        self.voice.strike(x);
    }
    fn note_off(&mut self, _note: u32) {}
    fn choke(&mut self) {
        self.voice.choke();
    }
    fn leave(&mut self) {
        self.voice.choke();
    }
    fn control_rate(&mut self) {}
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.voice.process(l, r);
    }
    fn has_output(&self) -> bool {
        self.voice.active()
    }
}
