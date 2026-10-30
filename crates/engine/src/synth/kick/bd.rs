//! The Kick's modal engines (Rasper, Snappy, Bold, Maple, Push): DECKARD's own designs on
//! [`modal_lite`](crate::synth::modal_lite), tuned against Maschine's renders (not ports;
//! docs/synth-kick.md §4).
//!
//! Every one is a falling sine (the body) with a couple of shell modes and a click; each
//! engine adds its own layer: Rasper a "crispness" noise, Snappy a snap click and a
//! punch transient, Maple a struck skin with a room. Measured in Maschine at the
//! defaults: Tune and the note are one control (Tune 0 plays as note 48, 1 as note 72),
//! the pitch rising ×1.69 per 12 steps; Decay stretches the tail (−60 dB at 250…510 ms
//! on Bold); Impact sets the attack's level, Punch its brightness.

use crate::synth::modal_lite::{Noise, Osc, Patch, Source, Voice, expmap, lin, vel_gain};

#[derive(Clone, Copy)]
pub enum Kind {
    Jungle,
    Medio,
    SubSnappy,
    Acoustic,
    Pusher,
}

/// One engine's sound at the default settings (Tune 0.5 on note 60, Decay 0.5).
struct Profile {
    /// Where the body settles and starts (Hz), and how fast it falls (s).
    f_end: f32,
    f_start: f32,
    fall: f32,
    /// Level held (s) and the tail's time constant at Decay 0 and 1 (s).
    hold: f32,
    decay: (f32, f32),
    drive: f32,
    /// Shell modes: (ratio to the body, level, decay as a share of the tail's).
    modes: &'static [(f32, f32, f32)],
    /// The click's level and brightness (Hz).
    click: (f32, f32),
    gain: f32,
}

const JUNGLE: Profile = Profile {
    f_end: 79.0,
    f_start: 210.0,
    fall: 0.022,
    hold: 0.004,
    decay: (0.042, 0.1),
    drive: 0.0,
    modes: &[(1.75, 0.75, 0.6), (0.71, 0.45, 0.9), (1.56, 0.4, 0.5)],
    click: (1.6, 2500.0),
    gain: 0.36,
};

const MEDIO: Profile = Profile {
    f_end: 64.0,
    f_start: 200.0,
    fall: 0.018,
    hold: 0.006,
    decay: (0.05, 0.11),
    drive: 0.0,
    modes: &[(1.61, 0.75, 0.5), (1.37, 0.12, 0.4)],
    click: (1.0, 3500.0),
    gain: 0.47,
};

const SUB_SNAPPY: Profile = Profile {
    f_end: 67.0,
    f_start: 130.0,
    fall: 0.02,
    hold: 0.015,
    decay: (0.04, 0.08),
    drive: 0.0,
    modes: &[(1.79, 0.55, 0.85)],
    click: (0.12, 1800.0),
    gain: 0.36,
};

const ACOUSTIC: Profile = Profile {
    f_end: 45.0,
    f_start: 100.0,
    fall: 0.03,
    hold: 0.0,
    decay: (0.05, 0.12),
    drive: 0.0,
    modes: &[(1.49, 1.4, 0.9), (2.67, 2.0, 0.6), (3.0, 0.4, 0.3), (1.76, 0.25, 0.4)],
    click: (0.8, 1500.0),
    gain: 0.25,
};

const PUSHER: Profile = Profile {
    f_end: 53.0,
    f_start: 200.0,
    fall: 0.016,
    hold: 0.06,
    decay: (0.05, 0.1),
    drive: 0.0,
    modes: &[(2.09, 0.7, 0.75), (1.72, 0.35, 0.7)],
    click: (0.25, 2200.0),
    gain: 0.5,
};

/// One modal Kick engine.
pub struct Bd {
    kind: Kind,
    voice: Voice,
    /// The core's parameters as set (core numbering: see [`Bd::set`]).
    p: [f32; 8],
}

impl Bd {
    pub fn new(kind: Kind, sr: f32) -> Bd {
        // Defaults: tune 0.5, decay 0.5, impact 0.75, punch 1, the engine's own at
        // their stored defaults, velocity scale 1.
        let p = match kind {
            Kind::Jungle => [0.5, 0.5, 0.75, 1.0, 0.34, 0.5, 0.0, 1.0],
            Kind::Medio => [0.5, 0.5, 0.75, 0.5, 1.0, 0.5, 0.0, 1.0],
            Kind::SubSnappy | Kind::Pusher => [0.5, 0.5, 0.75, 1.0, 1.0, 0.0, 0.0, 0.0],
            Kind::Acoustic => [0.5, 0.5, 0.5, 0.75, 0.5, 0.75, 0.0, 1.0],
        };
        Bd { kind, voice: Voice::new(sr), p }
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.voice.reset();
    }

    /// `setParameter(i, v)` in the core's numbering:
    /// - Rasper: 0 tune, 1 decay, 2 impact, 3 punch, 4 crispness decay, 5 amount,
    ///   6 mode, 7 velocity;
    /// - Snappy: 0 tune, 1 decay, 2 impact, 3 snap, 4 punch amount, 5 punch decay,
    ///   6 punch mode, 7 velocity;
    /// - Bold, Push: 0 tune, 1 decay, 2 impact, 3 punch, 4 velocity;
    /// - Maple: 0 skin tune, 1 tune, 2 decay, 3 impact, 4 room size, 5 room amount,
    ///   6 room character, 7 velocity.
    pub fn set(&mut self, i: usize, v: f32) {
        if let Some(x) = self.p.get_mut(i) {
            *x = v;
        }
    }

    pub fn note_on(&mut self, vel: f32, note: u32) {
        let patch = self.patch(vel, note);
        self.voice.strike(patch);
    }

    #[allow(dead_code)]
    pub fn note_off(&mut self, _note: u32) {}

    pub fn choke(&mut self) {
        self.voice.choke();
    }

    pub fn control(&mut self) {}

    pub fn active(&self) -> bool {
        self.voice.active()
    }

    /// Renders into `l`/`r`; returns whether it sounds.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) -> bool {
        let on = self.voice.active();
        self.voice.process(l, r);
        on
    }

    /// The strike for `vel` (0..1) on `note`.
    fn patch(&self, vel: f32, note: u32) -> Patch {
        let p = &self.p;
        let (prof, tune, decay, impact, punch, velocity) = match self.kind {
            Kind::Jungle => (&JUNGLE, p[0], p[1], p[2], p[3], p[7]),
            Kind::Medio => (&MEDIO, p[0], p[1], p[2], 0.6, p[7]),
            Kind::SubSnappy => (&SUB_SNAPPY, p[0], p[1], p[2], p[3], p[4]),
            Kind::Pusher => (&PUSHER, p[0], p[1], p[2], p[3], p[4]),
            Kind::Acoustic => (&ACOUSTIC, p[1], p[2], p[3], 0.5, p[7]),
        };
        // Tune and the note are one pitch control: ±1 step per semitone, ×1.69 per 12.
        let steps = (tune - 0.5) * 24.0 + (note as f32 - 60.0);
        let k = 1.69f32.powf(steps / 12.0);
        let tail = expmap(decay, prof.decay.0, prof.decay.1);
        let level = vel_gain(vel, velocity) * prof.gain;
        let mut x = Patch::default();
        // Impact lifts the strike: a higher start and a stronger click.
        let attack = lin(impact, 0.55, 1.35);
        x.osc = Osc {
            level: 1.0,
            f_start: prof.f_start * k * lin(impact, 0.85, 1.15),
            f_end: prof.f_end * k,
            fall: prof.fall,
            hold: prof.hold,
            decay: tail,
            drive: prof.drive,
        };
        for &(ratio, lvl, dec) in prof.modes {
            x.mode(prof.f_end * k * ratio, lvl, tail * dec, 0.0);
        }
        x.bend = 0.6;
        x.bend_fall = prof.fall;
        x.click = Noise {
            level: prof.click.0 * attack * lin(punch, 0.3, 1.0),
            source: Source::White,
            freq: prof.click.1 * lin(punch, 0.6, 1.4),
            q: 0.8,
            hp: 60.0,
            attack: 0.0,
            decay: 0.004,
            width: 0.0,
        };
        match self.kind {
            Kind::Jungle => {
                // Crispness: a bright noise tail (mode B darker and longer).
                let b = p[6] >= 0.5;
                x.noise = Noise {
                    level: p[5] * 0.12,
                    source: Source::Modal(if b { 9 } else { 0 }),
                    freq: if b { 2200.0 } else { 6000.0 },
                    q: 0.7,
                    hp: 400.0,
                    attack: 0.001,
                    decay: expmap(p[4], 0.01, 0.25),
                    width: 0.0,
                };
            }
            Kind::Medio => {
                // Snap: a sharper click; Punch: a short low-mid knock.
                x.click.level = prof.click.0 * attack * lin(p[3], 0.2, 1.3);
                x.click.freq = lin(p[3], 1500.0, 6000.0);
                let b = p[6] >= 0.5;
                x.mode(prof.f_end * k * if b { 2.6 } else { 3.4 }, p[4] * 0.25, expmap(p[5], 0.01, 0.08), 0.0);
            }
            Kind::Acoustic => {
                // Skin Tune moves the skin modes against the body; the room is a soft
                // pink tail (character B brighter).
                let skin = (2f32).powf((p[0] - 0.5) * 1.5);
                for m in x.modes[..x.n_modes].iter_mut() {
                    m.freq *= skin;
                }
                let b = p[6] >= 0.5;
                x.noise = Noise {
                    level: p[5] * 0.05,
                    source: Source::Pink,
                    freq: if b { 1800.0 } else { 600.0 },
                    q: 0.5,
                    hp: 80.0,
                    attack: 0.01,
                    decay: expmap(p[4], 0.05, 0.6),
                    width: 0.0,
                };
            }
            _ => {}
        }
        x.gain = level * attack.sqrt();
        x
    }
}
