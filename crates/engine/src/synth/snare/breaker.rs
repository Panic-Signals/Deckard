//! The Snare's Breaker engine (`NI::DSP::BreakerSDCore` in Maschine): DECKARD's own design,
//! tuned against Maschine's renders of each knob's extremes (docs/drum-engines-own.md).
//!
//! What makes it a snare rather than a tuned drum, as measured there:
//!
//! - low modes that follow Tune and the note, which are one control;
//! - resonances that stay where they are whatever the tuning (1.3 to 4.3 kHz), gone within
//!   a few milliseconds: the crack;
//! - wires: noise from about 1 kHz up to the top of the range, under the whole strike, even
//!   with Spectra at 0;
//! - Spectra: noise ringing resonators at their own pitches, coming in about 15 ms later.

use super::modal::{Kind, ModalSnare, hardness};
use crate::synth::modal_lite::{Noise, Patch, Source, expmap, lin};

pub(crate) struct Breaker;

impl Breaker {
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(sr: f32) -> ModalSnare {
        ModalSnare::new(Kind::Breaker, sr)
    }
}

/// A mode that follows the tuning.
struct Tuned {
    /// Hz at the middle of the tuning, and dB.
    freq: f32,
    level: f32,
    /// Its decay against the body's.
    share: f32,
    /// How far Skin Tune below its middle lifts it (a ratio).
    skin: f32,
    /// It stays put when the tuning goes below the middle.
    only_up: bool,
    /// dB it's lifted by at the softest strike: it's what's left of the tone there.
    soft: f32,
}

const fn tuned(freq: f32, level: f32, share: f32, skin: f32, only_up: bool, soft: f32) -> Tuned {
    Tuned { freq, level, share, skin, only_up, soft }
}

const TUNED: [Tuned; 8] = [
    tuned(273.0, -1.5, 1.0, 1.0, false, 0.0),
    tuned(173.0, -15.0, 0.2, 1.0, false, 0.0),
    tuned(386.0, -14.0, 0.95, 1.106, false, 20.0),
    tuned(236.0, -25.0, 0.6, 1.0, false, 0.0),
    tuned(447.0, -10.0, 0.43, 1.055, true, 0.0),
    tuned(344.0, -32.0, 0.6, 1.0, false, 0.0),
    tuned(501.0, -33.0, 0.8, 1.0, false, 0.0),
    tuned(787.0, -30.0, 1.0, 1.0, false, 0.0),
];

/// Modes that don't: Hz, dB, decay (s).
const FIXED: [(f32, f32, f32); 14] = [
    (615.0, -30.0, 0.012),
    (712.0, -30.0, 0.012),
    (923.0, -24.0, 0.008),
    (1084.0, -30.0, 0.008),
    (1344.0, -12.0, 0.005),
    (1422.0, -22.0, 0.005),
    (1511.0, -20.0, 0.005),
    (1666.0, -15.0, 0.005),
    (1782.0, -13.0, 0.005),
    (1854.0, -21.0, 0.005),
    (2238.0, -20.0, 0.004),
    (2473.0, -22.0, 0.004),
    (3167.0, -24.0, 0.004),
    (4328.0, -22.0, 0.003),
];

/// Spectra's resonators at the middle of Spectra Tune, for its two modes.
const SPECTRA_A: [(f32, f32); 9] =
    [(304.0, 0.5), (338.0, 1.0), (578.0, 1.8), (632.0, 2.4), (726.0, 1.5), (806.0, 2.0), (997.0, 1.5), (1770.0, 0.6), (3431.0, 0.6)];
const SPECTRA_B: [(f32, f32); 7] = [(529.0, 1.2), (575.0, 1.4), (811.0, 1.2), (949.0, 1.4), (1265.0, 0.6), (2359.0, 0.7), (2519.0, 0.6)];

fn db(x: f32) -> f32 {
    10f32.powf(x / 20.0)
}

/// `p` by meaning: tune, decay, skin tune, impact, body, spectra mode, spectra tune, spectra
/// amount, spectra decay, velocity.
pub(super) fn patch(p: &[f32; 10], vel: f32, note: u32) -> Patch {
    let [tune, decay, skin, impact, _, smode, stune, samount, sdecay, velocity] = *p;
    // Tune and the note are one control, an octave each way.
    let tuned = (tune + (note as f32 - 60.0) / 24.0).clamp(0.0, 1.0);
    let k = ((tuned - 0.5) * 2.0).exp2();
    // The tone is at its loudest in the middle.
    let tone = if tuned < 0.5 { 0.0 } else { lin(tuned * 2.0 - 1.0, 0.0, -7.0) };
    let hard = hardness(vel, velocity);
    // A soft strike loses the tone first, and the wires last: each part's level falls away
    // with its own curve (fitted to velocities 30, 100 and 127).
    let soft = 1.0 - hard;
    let curve = |a: f32, b: f32| -(a * soft + b * soft * soft);
    let body_db = curve(18.9, 66.5) + 4.1 + lin(impact, -2.6, 1.4) - lin(0.75, -2.6, 1.4);
    // Above the middle of the tuning, the wires are lower and shorter.
    let high = (tuned * 2.0 - 1.0).max(0.0);
    let wires_db = curve(25.4, 26.7) + 4.2 - 5.0 * high;
    let crack_db = curve(47.9, 28.7) + 14.5 + (impact - 0.75) * 12.0;
    let tail = if decay < 0.5 { lin(decay * 2.0, 0.03, 0.052) } else { lin(decay * 2.0 - 1.0, 0.052, 0.095) };
    // Lower, the tone rings longer.
    let tail = tail * (1.0 + 0.3 * (1.0 - tuned * 2.0).max(0.0));
    let mut x = Patch::default();
    let lift = (1.0 - skin * 2.0).max(0.0);
    for m in &TUNED {
        let follow = if m.only_up { k.max(1.0) } else { k };
        let f = m.freq * follow * (1.0 + (m.skin - 1.0) * lift);
        x.mode(f, db(m.level + tone + body_db + m.soft * soft), tail * m.share, 0.0);
    }
    // (At scattered phases: all together they'd make a peak Maschine's strike doesn't have.)
    x.scatter_from = x.n_modes;
    for (f, level, decay) in FIXED {
        x.mode(f, db(level + crack_db), decay, 0.0);
    }
    // The strike: a burst across the range.
    x.click =
        Noise { level: 0.1 * db(crack_db), source: Source::White, freq: 2800.0, q: 0.9, hp: 800.0, attack: 0.0, decay: 0.003, width: 0.0 };
    // The wires, under everything.
    x.noise = Noise {
        level: 1.25 * db(wires_db),
        source: Source::Pink,
        freq: 4200.0,
        q: 0.42,
        hp: 450.0,
        attack: 0.001,
        decay: lin(decay, 0.034, 0.05) * (1.0 - 0.2 * high),
        width: 0.0,
    };
    // Spectra, coming in after the strike.
    let sk = ((stune - 0.5) * 1.66).exp2();
    let rings: &[(f32, f32)] = if smode >= 0.5 { &SPECTRA_B } else { &SPECTRA_A };
    x.bank.q = 14.0;
    for &(f, gain) in rings {
        x.bank.ring(f * sk, gain);
    }
    x.bank.level = 0.8 * samount.powf(1.2) * db(wires_db);
    x.bank.attack = 0.008;
    x.bank.decay = expmap(sdecay, 0.012, 0.085);
    x.gain = 0.15;
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Velocity follows at 1, is full at 0 and inverted at −1.
    #[test]
    fn the_velocity_setting() {
        assert_eq!(hardness(1.0, 1.0), 1.0);
        assert!((hardness(0.25, 0.0) - 1.0).abs() < 1e-6);
        let soft = hardness(0.25, 1.0);
        assert!(soft < 0.3, "{soft}");
        assert!((hardness(0.25, -1.0) - (1.0 - soft)).abs() < 1e-6);
    }

    /// Tune and the note are one control.
    #[test]
    fn tune_and_note_are_one() {
        let at = |tune: f32, note: u32| {
            let mut p = [0.5, 0.5, 0.5, 0.75, 0.75, 0.0, 0.5, 0.75, 0.4, 1.0];
            p[0] = tune;
            patch(&p, 0.8, note).modes[0].freq
        };
        assert_eq!(at(0.0, 60), at(0.5, 48));
        assert_eq!(at(1.0, 60), at(0.5, 72));
        assert_eq!(at(0.5, 60), TUNED[0].freq);
        assert_eq!(at(0.5, 100), at(1.0, 60), "no further than an octave");
    }

    /// Whatever the tuning, the crack stays where it is, and the wires are there with
    /// Spectra at 0.
    #[test]
    fn the_crack_and_the_wires_are_fixed() {
        let mut p = [0.5, 0.5, 0.5, 0.75, 0.75, 0.0, 0.5, 0.0, 0.4, 1.0];
        let a = patch(&p, 0.8, 60);
        p[0] = 1.0;
        let b = patch(&p, 0.8, 60);
        assert_eq!(a.modes[TUNED.len()].freq, b.modes[TUNED.len()].freq);
        assert!(b.modes[0].freq > a.modes[0].freq);
        assert!(a.noise.level > 0.0);
        assert_eq!(a.bank.level, 0.0);
    }
}
