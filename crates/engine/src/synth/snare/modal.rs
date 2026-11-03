//! The Snare's modal engines (Airy, Vintage, Chrome, Iron, Breaker): DECKARD's own designs on
//! [`modal_lite`](crate::synth::modal_lite), measured against Maschine's renders (not ports;
//! docs/drum-engines-own.md).
//!
//! All of them are built the same way, as Breaker's measurements first showed:
//!
//! - modes that follow Tune and the note, which are one control; Skin Tune moves some;
//! - modes that stay where they are whatever the tuning, gone quickly: the crack;
//! - wires: noise under the whole strike, whatever Spectra is at;
//! - Spectra: noise ringing resonators at their own pitches;
//! - a soft strike that loses each part on its own curve.

use super::Core;
use super::designs::{AIRY, CHROME, IRON, VINTAGE};
use crate::synth::modal_lite::{Noise, Patch, Source, Voice, expmap, lin};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Airy,
    Vintage,
    Chrome,
    Iron,
    Breaker,
}

/// How a part's level answers the strike (dB).
pub(super) struct Part {
    /// How fast it falls with a softer strike.
    pub(super) a: f32,
    pub(super) b: f32,
    pub(super) offset: f32,
    /// Per unit of Impact from its default.
    pub(super) impact: f32,
}

impl Part {
    fn db(&self, soft: f32, impact: f32) -> f32 {
        -(self.a * soft + self.b * soft * soft) + self.offset + (impact - 0.75) * self.impact
    }
}

/// An engine's sound.
pub(super) struct Design {
    /// Semitones Tune spans each way.
    pub(super) span: f32,
    /// Modes that follow the tuning: Hz at its middle, dB, decay (s), moved by Skin Tune, and
    /// how far they follow it (1: all the way; a body's main mode can hardly move).
    pub(super) tuned: &'static [(f32, f32, f32, bool, f32)],
    /// Modes that don't: Hz, dB, decay (s).
    pub(super) fixed: &'static [(f32, f32, f32)],
    /// Spectra's resonators at the middle of Spectra Tune for its modes A and B: Hz, gain.
    pub(super) spectra: [&'static [(f32, f32)]; 2],
    /// dB on the tone at the bottom and the top of the tuning.
    pub(super) tone: (f32, f32),
    /// Decay at the knob's ends against its middle.
    pub(super) tail: (f32, f32),
    /// How much longer the tone rings at the bottom of the tuning.
    pub(super) tail_low: f32,
    /// Octaves Skin Tune spans.
    pub(super) skin: f32,
    pub(super) body: Part,
    pub(super) wires: Part,
    /// dB on the wires at the top of the tuning.
    pub(super) wires_high: f32,
    pub(super) crack: Part,
    /// The click: dB against the crack, band-pass Hz and Q, high-pass Hz, decay (s).
    pub(super) click: (f32, f32, f32, f32, f32),
    /// The wires: dB against their part, band-pass Hz and Q, high-pass Hz.
    pub(super) noise: (f32, f32, f32, f32),
    pub(super) noise_attack: f32,
    /// Their decay at Decay 0 and 1 (s).
    pub(super) noise_decay: (f32, f32),
    /// Spectra: dB against the wires, the resonators' Q, attack (s).
    pub(super) ring_level: f32,
    pub(super) ring_q: f32,
    pub(super) ring_attack: f32,
    /// Its decay at Spectra Decay 0 and 1 (s), how Amount bends, the octaves Spectra Tune spans.
    pub(super) ring_decay: (f32, f32),
    pub(super) ring_amount: f32,
    pub(super) ring_tune: f32,
    /// The strike's pitch bend (octaves at the default Impact) and its fall (s).
    pub(super) bend: f32,
    pub(super) bend_fall: f32,
    /// The modes come in with this time constant (s).
    pub(super) swell: f32,
}

/// Sharp's body and wires, under the attack it plays as Maschine does (`super::disco`).
pub(super) struct Sharp {
    pub(super) tuned: &'static [(f32, f32, f32, bool)],
    pub(super) fixed: &'static [(f32, f32, f32)],
    /// The modes come in with this time constant (s): the body starts after the attack.
    pub(super) swell: f32,
    pub(super) tail: (f32, f32),
    pub(super) skin: f32,
    pub(super) tone: (f32, f32),
    /// Its fall with a softer strike (two terms), dB, dB per unit of Body from its default.
    pub(super) body: (f32, f32, f32, f32),
    /// Its fall with a softer strike (two terms), dB, dB per unit of Noise from its default.
    pub(super) wires: (f32, f32, f32, f32),
    pub(super) noise: (f32, f32, f32, f32),
    pub(super) noise_attack: f32,
    /// Seconds, octaves per unit of Noise, octaves per unit of Decay (both from the default).
    pub(super) noise_decay: (f32, f32, f32),
}

fn db(x: f32) -> f32 {
    10f32.powf(x / 20.0)
}

/// How hard the strike is, 0..1, from the velocity under the Velocity setting (−1..1): at 1
/// it follows the velocity, at 0 it's full, at −1 it's inverted. The velocity reaches its
/// top at 0.88 and bends a little.
pub(super) fn hardness(vel: f32, setting: f32) -> f32 {
    let x = (vel.clamp(0.0, 1.0) / 0.88).min(1.0);
    let x = x * (0.6 + 0.4 * x);
    let s = setting.clamp(-1.0, 1.0) + 1.0;
    let a = s.min(1.0);
    let y = 1.0 - x + a * x;
    (y + (s - a) * (x - y)).clamp(0.0, 1.0)
}

/// A strike of `d`. `p` by meaning: tune, decay, skin tune, impact, body, spectra mode, spectra
/// tune, spectra amount, spectra decay, velocity.
fn strike(d: &Design, p: &[f32; 10], vel: f32, note: u32) -> Patch {
    let [tune, decay, skin, impact, _, smode, stune, samount, sdecay, velocity] = *p;
    let tuned = (tune + (note as f32 - 60.0) / (2.0 * d.span)).clamp(0.0, 1.0);
    let k = ((tuned - 0.5) * d.span / 6.0).exp2();
    let high = (tuned * 2.0 - 1.0).max(0.0);
    let low = (1.0 - tuned * 2.0).max(0.0);
    let soft = 1.0 - hardness(vel, velocity);
    let body_db = d.body.db(soft, impact) + d.tone.1 * high + d.tone.0 * low;
    let wires_db = d.wires.db(soft, impact) + d.wires_high * high;
    let crack_db = d.crack.db(soft, impact);
    let tail = if decay < 0.5 { lin(decay * 2.0, d.tail.0, 1.0) } else { lin(decay * 2.0 - 1.0, 1.0, d.tail.1) };
    let tail = tail * (1.0 + d.tail_low * low);
    let skin_k = ((skin - 0.5) * d.skin).exp2();
    let mut x = Patch::default();
    for &(f, level, tau, moves, follow) in d.tuned {
        x.mode(f * k.powf(follow) * if moves { skin_k } else { 1.0 }, db(level + body_db), tau * tail, 0.0);
    }
    // (At scattered phases: all together they'd make a peak Maschine's strike doesn't have.)
    x.scatter_from = x.n_modes;
    for &(f, level, tau) in d.fixed {
        x.mode(f, db(level + crack_db), tau, 0.0);
    }
    x.bend = d.bend * impact / 0.75;
    x.bend_fall = d.bend_fall;
    x.swell = d.swell;
    let (level, freq, q, hp, click_decay) = d.click;
    x.click = Noise { level: db(level + crack_db), source: Source::White, freq, q, hp, attack: 0.0, decay: click_decay, width: 0.0 };
    let (level, freq, q, hp) = d.noise;
    x.noise = Noise {
        level: db(level + wires_db),
        source: Source::Pink,
        freq,
        q,
        hp,
        attack: d.noise_attack,
        decay: lin(decay, d.noise_decay.0, d.noise_decay.1),
        width: 0.0,
    };
    let sk = ((stune - 0.5) * d.ring_tune).exp2();
    x.bank.q = d.ring_q;
    for &(f, gain) in d.spectra[usize::from(smode >= 0.5)] {
        x.bank.ring(f * sk, gain);
    }
    x.bank.level = samount.powf(d.ring_amount) * db(wires_db + d.ring_level);
    x.bank.attack = d.ring_attack;
    x.bank.decay = expmap(sdecay, d.ring_decay.0, d.ring_decay.1);
    x
}

/// Sharp's body and wires: `tune` is where the note lands on the tuning (0..1).
pub(super) fn sharp_strike(d: &Sharp, tune: f32, decay: f32, skin: f32, body: f32, noise: f32, hard: f32) -> Patch {
    let k = (tune - 0.5).exp2();
    let high = (tune * 2.0 - 1.0).max(0.0);
    let low = (1.0 - tune * 2.0).max(0.0);
    let soft = 1.0 - hard;
    let body_db = -(d.body.0 * soft + d.body.1 * soft * soft) + d.body.2 + (body - 0.75) * d.body.3 + d.tone.1 * high + d.tone.0 * low;
    let wires_db = -(d.wires.0 * soft + d.wires.1 * soft * soft) + d.wires.2 + (noise - 0.4) * d.wires.3;
    let tail = if decay < 0.5 { lin(decay * 2.0, d.tail.0, 1.0) } else { lin(decay * 2.0 - 1.0, 1.0, d.tail.1) };
    let skin_k = ((skin - 0.5) * d.skin).exp2();
    let mut x = Patch::default();
    for &(f, level, tau, moves) in d.tuned {
        x.mode(f * k * if moves { skin_k } else { 1.0 }, db(level + body_db), tau * tail, 0.0);
    }
    x.scatter_from = x.n_modes;
    for &(f, level, tau) in d.fixed {
        x.mode(f, db(level + wires_db + 22.0), tau, 0.0);
    }
    x.swell = d.swell;
    let (level, freq, q, hp) = d.noise;
    let (seconds, per_noise, per_decay) = d.noise_decay;
    x.noise = Noise {
        level: db(level + wires_db),
        source: Source::Pink,
        freq,
        q,
        hp,
        attack: d.noise_attack,
        decay: seconds * ((noise - 0.4) * per_noise + (decay - 0.5) * per_decay).exp2(),
        width: 0.0,
    };
    x
}

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

    fn patch(&self, vel: f32, note: u32) -> Patch {
        let d = match self.kind {
            Kind::Airy => &AIRY,
            Kind::Vintage => &VINTAGE,
            Kind::Chrome => &CHROME,
            Kind::Iron => &IRON,
            Kind::Breaker => return super::breaker::patch(&self.p, vel, note),
        };
        strike(d, &self.p, vel, note)
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

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULTS: [f32; 10] = [0.5, 0.5, 0.5, 0.75, 0.75, 0.0, 0.5, 0.75, 0.4, 1.0];

    fn each() -> [&'static Design; 4] {
        [&AIRY, &VINTAGE, &CHROME, &IRON]
    }

    /// Tune and the note are one control, as far as Tune goes.
    #[test]
    fn tune_and_note_are_one() {
        for d in each() {
            let semis = d.span as u32;
            let mut p = DEFAULTS;
            p[0] = 1.0;
            let a = strike(d, &p, 0.8, 60);
            let b = strike(d, &DEFAULTS, 0.8, 60 + semis);
            assert_eq!(a.modes[0].freq, b.modes[0].freq);
            assert_eq!(strike(d, &DEFAULTS, 0.8, 120).modes[0].freq, a.modes[0].freq, "no further");
        }
    }

    /// The crack stays where it is whatever the tuning.
    #[test]
    fn the_crack_is_fixed() {
        for d in each() {
            let mut p = DEFAULTS;
            p[0] = 1.0;
            let (a, b) = (strike(d, &DEFAULTS, 0.8, 60), strike(d, &p, 0.8, 60));
            let at = d.tuned.len();
            assert_eq!(a.modes[at].freq, b.modes[at].freq);
        }
    }

    /// Wires sound with Spectra off, and each Spectra mode rings something of its own.
    #[test]
    fn wires_and_spectra() {
        for d in each() {
            let mut p = DEFAULTS;
            p[7] = 0.0;
            let off = strike(d, &p, 0.8, 60);
            assert!(off.noise.level > 0.0);
            assert_eq!(off.bank.level, 0.0);
            assert!(!d.spectra[0].is_empty() && !d.spectra[1].is_empty());
            p[7] = 1.0;
            assert!(strike(d, &p, 0.8, 60).bank.level > 0.0);
        }
    }

    /// A soft strike is much quieter than a hard one.
    #[test]
    fn softer_is_quieter() {
        for d in each() {
            let (hard, soft) = (strike(d, &DEFAULTS, 1.0, 60), strike(d, &DEFAULTS, 0.2, 60));
            assert!(soft.modes[0].level < hard.modes[0].level * 0.5);
            assert!(soft.noise.level < hard.noise.level * 0.5);
        }
    }
}
