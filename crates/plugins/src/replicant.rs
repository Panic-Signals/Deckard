//! Replicant: DECKARD's subtractive synth, and the model for writing native engines
//! (see [`crate::native`]).
//!
//! Two band-limited oscillators (PolyBLEP saw and pulse, triangle, sine), a sub
//! oscillator an octave below oscillator 1, noise, a resonant state-variable filter
//! (low-, band- or high-pass) with its own envelope, an amp envelope, one LFO, 16
//! voices or one (legato, with glide), stereo spread and slow analog pitch drift.
//! Oscillator 1 can be a supersaw: up to seven detuned copies of its wave (after
//! Roland's JP-8000, with the detune spread and mix curves of Adam Szabo's analysis).

use crate::Transport;
use crate::native::{Class, Engine, Page, Param};

pub static CLASS: Class = Class {
    cid: *b"DECKARD:Replcnt1",
    name: "Replicant",
    instrument: true,
    params: PARAMS,
    pages: PAGES,
    make: |sr| Box::new(Replicant::new(sr)),
};

// Parameter ids (saved in projects: never renumber).
const OSC1_WAVE: u32 = 1;
const OSC2_WAVE: u32 = 2;
const OSC2_SEMI: u32 = 3;
const DETUNE: u32 = 4;
const MIX: u32 = 5;
const SUB: u32 = 6;
const NOISE: u32 = 7;
const PW: u32 = 8;
const F_TYPE: u32 = 10;
const CUTOFF: u32 = 11;
const RESO: u32 = 12;
const ENV_AMT: u32 = 13;
const F_ATTACK: u32 = 14;
const F_DECAY: u32 = 15;
const F_SUSTAIN: u32 = 16;
const F_RELEASE: u32 = 17;
const A_ATTACK: u32 = 20;
const A_DECAY: u32 = 21;
const A_SUSTAIN: u32 = 22;
const A_RELEASE: u32 = 23;
const VELOCITY: u32 = 24;
const KEY_TRACK: u32 = 25;
const GLIDE: u32 = 26;
const VOLUME: u32 = 27;
const LFO_RATE: u32 = 30;
const LFO_SHAPE: u32 = 31;
const LFO_PITCH: u32 = 32;
const LFO_CUTOFF: u32 = 33;
const MODE: u32 = 34;
const SPREAD: u32 = 35;
const DRIFT: u32 = 36;
const BEND: u32 = 37;
const SUPER_VOICES: u32 = 40;
const SUPER_DETUNE: u32 = 41;
const SUPER_MIX: u32 = 42;

const fn p(id: u32, name: &'static str, short: &'static str, default: f64, steps: i32) -> Param {
    Param { id, name, short, default, steps }
}

static PARAMS: &[Param] = &[
    p(OSC1_WAVE, "Osc 1 Wave", "WAVE 1", 0.0, 3),
    p(OSC2_WAVE, "Osc 2 Wave", "WAVE 2", 0.0, 3),
    p(OSC2_SEMI, "Osc 2 Pitch", "PITCH 2", 0.5, 48),
    p(DETUNE, "Detune", "DETUNE", 0.14, 0),
    p(MIX, "Osc Mix", "MIX", 0.5, 0),
    p(SUB, "Sub", "SUB", 0.0, 0),
    p(NOISE, "Noise", "NOISE", 0.0, 0),
    p(PW, "Pulse Width", "PW", 0.0, 0),
    p(F_TYPE, "Filter Type", "TYPE", 0.0, 2),
    p(CUTOFF, "Cutoff", "CUTOFF", 0.6, 0),
    p(RESO, "Resonance", "RESO", 0.2, 0),
    p(ENV_AMT, "Filter Env Amount", "ENV AMT", 0.7, 0),
    p(F_ATTACK, "Filter Attack", "ATTACK", 0.0, 0),
    p(F_DECAY, "Filter Decay", "DECAY", 0.55, 0),
    p(F_SUSTAIN, "Filter Sustain", "SUSTAIN", 0.3, 0),
    p(F_RELEASE, "Filter Release", "RELEASE", 0.5, 0),
    p(A_ATTACK, "Amp Attack", "ATTACK", 0.0, 0),
    p(A_DECAY, "Amp Decay", "DECAY", 0.6, 0),
    p(A_SUSTAIN, "Amp Sustain", "SUSTAIN", 0.8, 0),
    p(A_RELEASE, "Amp Release", "RELEASE", 0.55, 0),
    p(VELOCITY, "Velocity", "VELOCITY", 0.5, 0),
    p(KEY_TRACK, "Key Track", "KEY TRK", 0.5, 0),
    p(GLIDE, "Glide", "GLIDE", 0.0, 0),
    p(VOLUME, "Volume", "VOLUME", 36.0 / 42.0, 0),
    p(LFO_RATE, "LFO Rate", "RATE", 0.45, 0),
    p(LFO_SHAPE, "LFO Shape", "SHAPE", 0.0, 3),
    p(LFO_PITCH, "LFO to Pitch", "PITCH", 0.0, 0),
    p(LFO_CUTOFF, "LFO to Cutoff", "CUTOFF", 0.0, 0),
    p(MODE, "Voice Mode", "MODE", 0.0, 1),
    p(SPREAD, "Stereo Spread", "SPREAD", 0.3, 0),
    p(DRIFT, "Drift", "DRIFT", 0.2, 0),
    p(BEND, "Bend Range", "BEND", 2.0 / 12.0, 12),
    p(SUPER_VOICES, "Super Voices", "VOICES", 0.0, 3),
    p(SUPER_DETUNE, "Super Detune", "DETUNE", 0.5, 0),
    p(SUPER_MIX, "Super Mix", "MIX", 0.6, 0),
];

static PAGES: &[Page] = &[
    Page {
        name: "Oscillators",
        knobs: &[
            (OSC1_WAVE, "Oscillators"),
            (OSC2_WAVE, ""),
            (OSC2_SEMI, ""),
            (DETUNE, ""),
            (MIX, "Mixer"),
            (SUB, ""),
            (NOISE, ""),
            (PW, ""),
        ],
    },
    Page {
        name: "Filter",
        knobs: &[
            (F_TYPE, "Filter"),
            (CUTOFF, ""),
            (RESO, ""),
            (ENV_AMT, ""),
            (F_ATTACK, "Filter Envelope"),
            (F_DECAY, ""),
            (F_SUSTAIN, ""),
            (F_RELEASE, ""),
        ],
    },
    Page {
        name: "Amp",
        knobs: &[
            (A_ATTACK, "Amp Envelope"),
            (A_DECAY, ""),
            (A_SUSTAIN, ""),
            (A_RELEASE, ""),
            (VELOCITY, "Voice"),
            (KEY_TRACK, ""),
            (GLIDE, ""),
            (VOLUME, ""),
        ],
    },
    Page {
        name: "Modulation",
        knobs: &[
            (LFO_RATE, "LFO"),
            (LFO_SHAPE, ""),
            (LFO_PITCH, ""),
            (LFO_CUTOFF, ""),
            (MODE, "Character"),
            (SPREAD, ""),
            (DRIFT, ""),
            (BEND, ""),
        ],
    },
    Page { name: "Super", knobs: &[(SUPER_VOICES, "Supersaw (Osc 1)"), (SUPER_DETUNE, ""), (SUPER_MIX, "")] },
];

const WAVES: [&str; 4] = ["Saw", "Pulse", "Triangle", "Sine"];
const LFO_SHAPES: [&str; 4] = ["Sine", "Triangle", "Square", "S&H"];
const TYPES: [&str; 3] = ["Low Pass", "Band Pass", "High Pass"];
const VOICES: usize = 16;
/// Super Voices' choices: off (one oscillator), then copies of oscillator 1.
const SUPER_COUNTS: [usize; 4] = [1, 3, 5, 7];
/// The copies' frequency offsets at full detune, outermost first (Szabo's measurements
/// of the JP-8000); fewer copies take the ones nearest the centre's sides.
const SUPER_OFFSETS: [f64; 6] = [-0.110_023_13, 0.107_452_42, -0.062_884_39, 0.062_165_38, -0.019_523_56, 0.019_912_21];

/// The detune knob's curve (Szabo's fit of the JP-8000's).
fn super_detune(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    10028.7312891634 * v.powi(11) - 50818.8652045924 * v.powi(10) + 111363.4808729368 * v.powi(9) - 138150.6761080548 * v.powi(8)
        + 106649.6679158292 * v.powi(7)
        - 53046.9642751875 * v.powi(6)
        + 17019.9518580080 * v.powi(5)
        - 3425.0836591318 * v.powi(4)
        + 404.2703938388 * v.powi(3)
        - 24.1878824391 * v.powi(2)
        + 0.6717417634 * v
        + 0.0030115596
}

/// The centre copy's gain and each side copy's, for Mix `v` and `sides` side copies
/// (Szabo's gains for six, kept as loud with fewer).
fn super_gains(v: f64, sides: usize) -> (f64, f64) {
    let centre = -0.55366 * v + 0.99785;
    let side = -0.73764 * v * v + 1.2841 * v + 0.044372;
    (centre, side * (6.0 / sides.max(1) as f64).sqrt())
}
/// Samples between filter and modulation updates.
const CONTROL: usize = 16;

/// Envelope stage time: 1 ms .. 10 s.
fn env_time(v: f64) -> f64 {
    0.001 * 10_000f64.powf(v)
}

fn cutoff_hz(v: f64) -> f64 {
    20.0 * 1000f64.powf(v)
}

fn lfo_hz(v: f64) -> f64 {
    0.05 * 400f64.powf(v)
}

fn glide_time(v: f64) -> f64 {
    if v <= 0.0 { 0.0 } else { 0.002 * 1000f64.powf(v) }
}

fn step(v: f64, steps: usize) -> usize {
    (v * steps as f64).round() as usize
}

fn time_text(s: f64) -> String {
    if s < 1.0 { format!("{:.0} ms", s * 1000.0) } else { format!("{s:.2} s") }
}

fn hz_text(hz: f64) -> String {
    if hz >= 1000.0 {
        format!("{:.2} kHz", hz / 1000.0)
    } else if hz >= 10.0 {
        format!("{hz:.0} Hz")
    } else {
        format!("{hz:.2} Hz")
    }
}

/// The PolyBLEP correction around a discontinuity at phase 0 (`t` phase, `dt` step).
fn blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

fn osc(wave: usize, t: f64, dt: f64, pw: f64) -> f64 {
    match wave {
        0 => 2.0 * t - 1.0 - blep(t, dt),
        1 => {
            let naive = if t < pw { 1.0 } else { -1.0 };
            let t2 = (t - pw).rem_euclid(1.0);
            naive + blep(t, dt) - blep(t2, dt)
        }
        2 => 4.0 * (t - 0.5).abs() - 1.0,
        _ => (t * std::f64::consts::TAU).sin(),
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Stage {
    #[default]
    Off,
    Attack,
    Decay,
    Release,
}

/// An ADSR: linear attack, exponential decay and release.
#[derive(Clone, Copy, Default)]
struct Env {
    stage: Stage,
    level: f64,
}

#[derive(Clone, Copy, Default)]
struct EnvSettings {
    attack: f64,
    /// Per-sample multipliers of the distance to the target.
    decay: f64,
    sustain: f64,
    release: f64,
}

impl EnvSettings {
    fn new(a: f64, d: f64, s: f64, r: f64, sr: f64) -> EnvSettings {
        let coef = |t: f64| (-5.0 / (t * sr)).exp();
        EnvSettings { attack: 1.0 / (env_time(a) * sr), decay: coef(env_time(d)), sustain: s, release: coef(env_time(r)) }
    }
}

impl Env {
    fn gate(&mut self, on: bool) {
        self.stage = if on {
            Stage::Attack
        } else if self.stage == Stage::Off {
            Stage::Off
        } else {
            Stage::Release
        };
    }

    fn next(&mut self, s: &EnvSettings) -> f64 {
        match self.stage {
            Stage::Off => self.level = 0.0,
            Stage::Attack => {
                self.level += s.attack;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => self.level = s.sustain + (self.level - s.sustain) * s.decay,
            Stage::Release => {
                self.level *= s.release;
                if self.level < 1e-5 {
                    self.level = 0.0;
                    self.stage = Stage::Off;
                }
            }
        }
        self.level
    }
}

#[derive(Clone, Copy, Default)]
struct Voice {
    note: u8,
    gate: bool,
    /// When it started (to steal the oldest).
    age: u64,
    velocity: f64,
    /// The pitch now (semitones, gliding to `note`).
    pitch: f64,
    phase: [f64; 3],
    /// The supersaw's side copies of oscillator 1.
    super_phase: [f64; 6],
    amp: Env,
    filter: Env,
    /// Filter state and coefficients.
    ic1: f64,
    ic2: f64,
    g: f64,
    k: f64,
    pan: (f32, f32),
    drift: f64,
    drift_to: f64,
}

impl Voice {
    fn sounding(&self) -> bool {
        self.amp.stage != Stage::Off
    }
}

struct Settings {
    waves: [usize; 2],
    semi: f64,
    detune: f64,
    mix: f64,
    sub: f64,
    noise: f64,
    pw: f64,
    f_type: usize,
    cutoff: f64,
    reso: f64,
    env_amt: f64,
    filter_env: EnvSettings,
    amp_env: EnvSettings,
    velocity: f64,
    key_track: f64,
    glide: f64,
    volume: f64,
    lfo_rate: f64,
    lfo_shape: usize,
    lfo_pitch: f64,
    lfo_cutoff: f64,
    mono: bool,
    spread: f64,
    drift: f64,
    bend_range: f64,
    /// Copies of oscillator 1 besides itself (0: no supersaw), their spread and gains.
    super_sides: usize,
    super_detune: f64,
    super_gains: (f64, f64),
}

pub struct Replicant {
    sr: f64,
    values: std::collections::HashMap<u32, f64>,
    s: Settings,
    voices: [Voice; VOICES],
    clock: u64,
    /// Notes held in mono mode, oldest first.
    stack: Vec<u8>,
    last_pitch: Option<f64>,
    lfo_phase: f64,
    lfo_hold: f64,
    lfo: f64,
    bend: f64,
    mod_wheel: f64,
    rng: u32,
    /// Smoothed cutoff (octaves above 20 Hz) and volume.
    cutoff_now: f64,
    volume_now: f64,
    countdown: usize,
}

impl Replicant {
    pub fn new(sr: f64) -> Replicant {
        let mut r = Replicant {
            sr,
            values: PARAMS.iter().map(|p| (p.id, p.default)).collect(),
            s: Settings {
                waves: [0, 0],
                semi: 0.0,
                detune: 0.0,
                mix: 0.5,
                sub: 0.0,
                noise: 0.0,
                pw: 0.5,
                f_type: 0,
                cutoff: 0.6,
                reso: 0.0,
                env_amt: 0.0,
                filter_env: EnvSettings::default(),
                amp_env: EnvSettings::default(),
                velocity: 0.5,
                key_track: 0.5,
                glide: 0.0,
                volume: 1.0,
                lfo_rate: 1.0,
                lfo_shape: 0,
                lfo_pitch: 0.0,
                lfo_cutoff: 0.0,
                mono: false,
                spread: 0.0,
                drift: 0.0,
                bend_range: 2.0,
                super_sides: 0,
                super_detune: 0.0,
                super_gains: (1.0, 0.0),
            },
            voices: [Voice::default(); VOICES],
            clock: 0,
            stack: Vec::new(),
            last_pitch: None,
            lfo_phase: 0.0,
            lfo_hold: 0.0,
            lfo: 0.0,
            bend: 0.0,
            mod_wheel: 0.0,
            rng: 0x1234_5678,
            cutoff_now: f64::NAN,
            volume_now: f64::NAN,
            countdown: 0,
        };
        r.update();
        r
    }

    fn v(&self, id: u32) -> f64 {
        self.values.get(&id).copied().unwrap_or(0.0)
    }

    /// Derive the settings from the parameter values.
    fn update(&mut self) {
        let sr = self.sr;
        let v = |id| self.v(id);
        self.s = Settings {
            waves: [step(v(OSC1_WAVE), 3), step(v(OSC2_WAVE), 3)],
            semi: step(v(OSC2_SEMI), 48) as f64 - 24.0,
            detune: v(DETUNE) * 50.0,
            mix: v(MIX),
            sub: v(SUB),
            noise: v(NOISE),
            pw: 0.5 + 0.45 * v(PW),
            f_type: step(v(F_TYPE), 2),
            cutoff: v(CUTOFF),
            reso: v(RESO),
            env_amt: v(ENV_AMT) * 2.0 - 1.0,
            filter_env: EnvSettings::new(v(F_ATTACK), v(F_DECAY), v(F_SUSTAIN), v(F_RELEASE), sr),
            amp_env: EnvSettings::new(v(A_ATTACK), v(A_DECAY), v(A_SUSTAIN), v(A_RELEASE), sr),
            velocity: v(VELOCITY),
            key_track: v(KEY_TRACK),
            glide: glide_time(v(GLIDE)),
            volume: 10f64.powf((v(VOLUME) * 42.0 - 36.0) / 20.0),
            lfo_rate: lfo_hz(v(LFO_RATE)),
            lfo_shape: step(v(LFO_SHAPE), 3),
            lfo_pitch: v(LFO_PITCH),
            lfo_cutoff: v(LFO_CUTOFF),
            mono: step(v(MODE), 1) == 1,
            spread: v(SPREAD),
            drift: v(DRIFT),
            bend_range: step(v(BEND), 12) as f64,
            super_sides: SUPER_COUNTS[step(v(SUPER_VOICES), 3)] - 1,
            super_detune: super_detune(v(SUPER_DETUNE)),
            super_gains: super_gains(v(SUPER_MIX), SUPER_COUNTS[step(v(SUPER_VOICES), 3)] - 1),
        };
    }

    fn random(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f64 / u32::MAX as f64 * 2.0 - 1.0
    }

    /// Start voice `i` on `note`.
    fn start(&mut self, i: usize, note: u8, velocity: u8, retrigger: bool) {
        let from = match self.last_pitch {
            Some(p) if self.s.glide > 0.0 => p,
            _ => note as f64,
        };
        let pan = self.random() * self.s.spread;
        let a = (pan + 1.0) * std::f64::consts::FRAC_PI_4;
        self.clock += 1;
        // The supersaw's copies start anywhere (free-running, as on the JP-8000).
        let phases: [f64; 6] = std::array::from_fn(|_| self.random() * 0.5 + 0.5);
        let v = &mut self.voices[i];
        if !v.sounding() {
            v.ic1 = 0.0;
            v.ic2 = 0.0;
            v.phase = [0.0; 3];
            v.super_phase = phases;
            v.drift = 0.0;
        }
        v.note = note;
        v.gate = true;
        v.age = self.clock;
        v.velocity = velocity as f64 / 127.0;
        v.pitch = from;
        v.pan = (a.cos() as f32 * std::f32::consts::SQRT_2, a.sin() as f32 * std::f32::consts::SQRT_2);
        if retrigger {
            v.amp.gate(true);
            v.filter.gate(true);
        }
        self.last_pitch = Some(note as f64);
    }

    /// A free voice, or the one to steal (released first, then the oldest).
    fn free_voice(&self) -> usize {
        if let Some(i) = self.voices.iter().position(|v| !v.sounding()) {
            return i;
        }
        let key = |v: &Voice| (v.gate, v.age);
        (0..VOICES).min_by_key(|&i| key(&self.voices[i])).unwrap_or(0)
    }

    fn lfo_value(&mut self, dt: f64) -> f64 {
        self.lfo_phase += dt;
        if self.lfo_phase >= 1.0 {
            self.lfo_phase -= self.lfo_phase.floor();
            self.lfo_hold = self.random();
        }
        let t = self.lfo_phase;
        match self.s.lfo_shape {
            0 => (t * std::f64::consts::TAU).sin(),
            1 => 1.0 - 4.0 * (t - 0.5).abs(),
            2 => {
                if t < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            _ => self.lfo_hold,
        }
    }

    /// Per-control-block updates: LFO, drift, glide, filter coefficients.
    fn control(&mut self) {
        let block = CONTROL as f64 / self.sr;
        self.lfo = self.lfo_value(self.s.lfo_rate * block);
        let target = self.s.cutoff * (1000f64).log2();
        self.cutoff_now = if self.cutoff_now.is_nan() { target } else { target + (self.cutoff_now - target) * 0.7 };
        self.volume_now = if self.volume_now.is_nan() { self.s.volume } else { self.s.volume + (self.volume_now - self.s.volume) * 0.7 };
        let glide = if self.s.glide > 0.0 { (-5.0 * block / self.s.glide).exp() } else { 0.0 };
        let nyquist = self.sr * 0.45;
        for i in 0..VOICES {
            if !self.voices[i].sounding() {
                continue;
            }
            let r = self.random();
            let v = &mut self.voices[i];
            v.pitch = v.note as f64 + (v.pitch - v.note as f64) * glide;
            // Drift: a slow random walk of up to ±12 cents.
            if r.abs() > 0.97 {
                v.drift_to = r * 0.12 * self.s.drift;
            }
            v.drift += (v.drift_to - v.drift) * 0.01;
            let key = (v.pitch - 60.0) / 12.0 * self.s.key_track;
            let env = v.filter.level * self.s.env_amt * 7.0;
            let lfo = self.lfo * self.s.lfo_cutoff * 4.0;
            let vel = (v.velocity - 1.0) * self.s.velocity * 1.5;
            let hz = (20.0 * (self.cutoff_now + key + env + lfo + vel).exp2()).clamp(20.0, nyquist);
            v.g = (std::f64::consts::PI * hz / self.sr).tan();
            v.k = 2.0 - 1.96 * self.s.reso;
        }
    }
}

impl Engine for Replicant {
    fn set_param(&mut self, id: u32, value: f64) {
        self.values.insert(id, value);
        self.update();
    }

    fn text(&self, id: u32, v: f64) -> String {
        match id {
            OSC1_WAVE | OSC2_WAVE => WAVES[step(v, 3)].into(),
            OSC2_SEMI => format!("{:+} st", step(v, 48) as i64 - 24),
            DETUNE => format!("{:.1} ct", v * 50.0),
            MIX => format!("{:.0} / {:.0}", (1.0 - v) * 100.0, v * 100.0),
            PW => format!("{:.0} %", (0.5 + 0.45 * v) * 100.0),
            F_TYPE => TYPES[step(v, 2)].into(),
            CUTOFF => hz_text(cutoff_hz(v)),
            ENV_AMT => format!("{:+.0} %", (v * 2.0 - 1.0) * 100.0),
            F_ATTACK | F_DECAY | F_RELEASE | A_ATTACK | A_DECAY | A_RELEASE => time_text(env_time(v)),
            GLIDE => {
                if v <= 0.0 {
                    "Off".into()
                } else {
                    time_text(glide_time(v))
                }
            }
            VOLUME => format!("{:+.1} dB", v * 42.0 - 36.0),
            LFO_RATE => hz_text(lfo_hz(v)),
            LFO_SHAPE => LFO_SHAPES[step(v, 3)].into(),
            LFO_PITCH => format!("{:.2} st", v * v * 12.0),
            MODE => ["Poly", "Mono"][step(v, 1)].into(),
            BEND => format!("{} st", step(v, 12)),
            SUPER_VOICES => match SUPER_COUNTS[step(v, 3)] {
                1 => "Off".into(),
                n => format!("{n}"),
            },
            SUPER_DETUNE => format!("{:.0} ct", 1200.0 * (1.0 + SUPER_OFFSETS[1] * super_detune(v)).log2()),
            _ => format!("{:.0} %", v * 100.0),
        }
    }

    fn note_on(&mut self, pitch: u8, velocity: u8) {
        if self.s.mono {
            let legato = !self.stack.is_empty();
            self.stack.retain(|&n| n != pitch);
            self.stack.push(pitch);
            let i = 0;
            for v in self.voices.iter_mut().skip(1) {
                v.gate = false;
                v.amp.gate(false);
                v.filter.gate(false);
            }
            self.start(i, pitch, velocity, !legato || !self.voices[0].sounding());
        } else {
            let i = self.free_voice();
            self.start(i, pitch, velocity, true);
        }
    }

    fn note_off(&mut self, pitch: u8) {
        if self.s.mono {
            self.stack.retain(|&n| n != pitch);
            let v = &mut self.voices[0];
            if v.note != pitch || !v.gate {
                return;
            }
            if let Some(&back) = self.stack.last() {
                // Legato back to the note still held.
                v.note = back;
                self.last_pitch = Some(back as f64);
                if self.s.glide <= 0.0 {
                    v.pitch = back as f64;
                }
                return;
            }
            v.gate = false;
            v.amp.gate(false);
            v.filter.gate(false);
            return;
        }
        for v in self.voices.iter_mut().filter(|v| v.gate && v.note == pitch) {
            v.gate = false;
            v.amp.gate(false);
            v.filter.gate(false);
        }
    }

    fn control_change(&mut self, number: u8, value: u8) {
        match number {
            1 => self.mod_wheel = value as f64 / 127.0,
            120 | 123 => self.reset(),
            _ => {}
        }
    }

    fn pitch_bend(&mut self, bend: f32) {
        self.bend = bend as f64;
    }

    fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
        self.stack.clear();
        self.last_pitch = None;
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], _transport: &Transport) {
        let sr = self.sr;
        for n in 0..left.len() {
            if self.countdown == 0 {
                self.control();
                self.countdown = CONTROL;
            }
            self.countdown -= 1;
            let s = &self.s;
            // Vibrato: the LFO's pitch depth, plus the mod wheel's.
            let vibrato = self.lfo * (s.lfo_pitch * s.lfo_pitch * 12.0 + self.mod_wheel * 0.5);
            let shift = vibrato + self.bend * s.bend_range;
            let (mut l, mut r) = (0.0f64, 0.0f64);
            for i in 0..VOICES {
                if !self.voices[i].sounding() {
                    continue;
                }
                let noise = if s.noise > 0.0 {
                    self.rng ^= self.rng << 13;
                    self.rng ^= self.rng >> 17;
                    self.rng ^= self.rng << 5;
                    self.rng as f64 / u32::MAX as f64 * 2.0 - 1.0
                } else {
                    0.0
                };
                let v = &mut self.voices[i];
                let pitch = v.pitch + shift + v.drift;
                let f1 = 440.0 * ((pitch - 69.0 - s.detune / 200.0) / 12.0).exp2();
                let f2 = 440.0 * ((pitch - 69.0 + s.semi + s.detune / 200.0) / 12.0).exp2();
                let dts = [f1 / sr, f2 / sr, f1 * 0.5 / sr];
                let mut o1 = osc(s.waves[0], v.phase[0], dts[0], s.pw);
                if s.super_sides > 0 {
                    let (centre, side) = s.super_gains;
                    let mut sides = 0.0;
                    for (k, ph) in v.super_phase.iter_mut().take(s.super_sides).enumerate() {
                        let dt = dts[0] * (1.0 + SUPER_OFFSETS[6 - s.super_sides + k] * s.super_detune);
                        sides += osc(s.waves[0], *ph, dt, s.pw);
                        *ph += dt.min(0.5);
                        if *ph >= 1.0 {
                            *ph -= 1.0;
                        }
                    }
                    o1 = o1 * centre + sides * side;
                }
                let o2 = osc(s.waves[1], v.phase[1], dts[1], s.pw);
                let sub = osc(1, v.phase[2], dts[2], 0.5);
                for (ph, dt) in v.phase.iter_mut().zip(dts) {
                    *ph += dt.min(0.5);
                    if *ph >= 1.0 {
                        *ph -= 1.0;
                    }
                }
                let x = o1 * (1.0 - s.mix) + o2 * s.mix + sub * s.sub + noise * s.noise;
                // A little drive into the filter.
                let x = (x * 0.8).tanh();
                // (The filter envelope's level moves the cutoff in `control`.)
                v.filter.next(&s.filter_env);
                let (g, k) = (v.g, v.k);
                let a1 = 1.0 / (1.0 + g * (g + k));
                let (a2, a3) = (g * a1, g * g * a1);
                let v3 = x - v.ic2;
                let v1 = a1 * v.ic1 + a2 * v3;
                let v2 = v.ic2 + a2 * v.ic1 + a3 * v3;
                v.ic1 = 2.0 * v1 - v.ic1;
                v.ic2 = 2.0 * v2 - v.ic2;
                let y = match s.f_type {
                    0 => v2,
                    1 => v1 * k,
                    _ => x - k * v1 - v2,
                };
                let amp = v.amp.next(&s.amp_env) * (1.0 - s.velocity + s.velocity * v.velocity);
                let out = y * amp;
                l += out * v.pan.0 as f64;
                r += out * v.pan.1 as f64;
                if !v.sounding() {
                    v.ic1 = 0.0;
                    v.ic2 = 0.0;
                }
            }
            let gain = 0.3 * self.volume_now;
            left[n] = (l * gain) as f32;
            right[n] = (r * gain) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::Instance;

    fn render(inst: &mut Instance, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let t = Transport::default();
        let (mut l, mut r) = (Vec::new(), Vec::new());
        let mut done = 0;
        while done < frames {
            let k = (frames - done).min(crate::MAX_BLOCK);
            let (a, b) = inst.process_input(k, &t, None);
            l.extend_from_slice(a);
            r.extend_from_slice(b);
            done += k;
        }
        (l, r)
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
    }

    #[test]
    fn plays_and_releases() {
        let mut inst = Instance::new(&CLASS, 48_000.0);
        assert_eq!(peak(&render(&mut inst, 4800).0), 0.0, "silent before a note");
        inst.note_on(60, 100);
        let (l, r) = render(&mut inst, 24_000);
        assert!(peak(&l) > 0.05 && peak(&r) > 0.05, "a held note sounds: {}", peak(&l));
        assert!(l.iter().chain(&r).all(|x| x.is_finite() && x.abs() < 4.0));
        inst.note_off(60);
        let (l, _) = render(&mut inst, 48_000);
        assert!(peak(&l[40_000..]) < 1e-4, "released note dies away");
    }

    /// A4 on a sine with the filter open plays 440 Hz (drift off).
    #[test]
    fn in_tune() {
        let mut inst = Instance::new(&CLASS, 48_000.0);
        for (id, v) in [(OSC1_WAVE, 1.0), (MIX, 0.0), (DETUNE, 0.0), (DRIFT, 0.0), (CUTOFF, 1.0), (ENV_AMT, 0.5), (RESO, 0.0)] {
            inst.set_param(id, v);
        }
        inst.note_on(69, 100);
        let (l, _) = render(&mut inst, 48_000);
        let x = &l[4800..];
        let ups = x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        let hz = ups as f64 * 48_000.0 / x.len() as f64;
        assert!((hz - 440.0).abs() < 1.5, "{hz} Hz");
    }

    /// The supersaw: off by default (one oscillator, as before); seven copies sound
    /// richer (more energy above the fundamental's harmonics' gaps) and stay in level.
    #[test]
    fn supersaw() {
        let render_with = |voices: f64| {
            let mut inst = Instance::new(&CLASS, 48_000.0);
            for (id, v) in [(DRIFT, 0.0), (DETUNE, 0.0), (MIX, 0.0), (CUTOFF, 1.0), (ENV_AMT, 0.5), (SUPER_VOICES, voices)] {
                inst.set_param(id, v);
            }
            inst.note_on(57, 100);
            render(&mut inst, 48_000).0
        };
        let plain = render_with(0.0);
        let full = render_with(1.0);
        let rms = |x: &[f32]| (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
        let (a, b) = (rms(&plain[4800..]), rms(&full[4800..]));
        assert!(b > a * 0.6 && b < a * 2.5, "level stays in range: {a} → {b}");
        // Detuned copies beat: the level changes over time; a lone saw's doesn't.
        let wobble = |x: &[f32]| {
            let parts: Vec<f64> = x[4800..].chunks(2400).map(rms).collect();
            let m = parts.iter().sum::<f64>() / parts.len() as f64;
            parts.iter().map(|p| (p - m).abs()).fold(0.0, f64::max) / m
        };
        assert!(wobble(&full) > wobble(&plain) + 0.01, "copies beat: {} vs {}", wobble(&full), wobble(&plain));
        let inst = Instance::new(&CLASS, 48_000.0);
        assert_eq!(inst.param_text(SUPER_VOICES, 0.0), "Off");
        assert_eq!(inst.param_text(SUPER_VOICES, 1.0), "7");
    }

    #[test]
    fn sustain_pedal_holds_notes() {
        let mut inst = Instance::new(&CLASS, 48_000.0);
        inst.control_change(64, 127);
        inst.note_on(64, 100);
        render(&mut inst, 4800);
        inst.note_off(64);
        let (l, _) = render(&mut inst, 48_000);
        assert!(peak(&l[40_000..]) > 0.01, "the pedal holds it");
        inst.control_change(64, 0);
        let (l, _) = render(&mut inst, 48_000);
        assert!(peak(&l[40_000..]) < 1e-4, "letting go releases it");
    }

    /// Every setting of every parameter renders finite audio (extremes included).
    #[test]
    fn every_extreme_is_stable() {
        for p in PARAMS {
            for v in [0.0, 1.0] {
                let mut inst = Instance::new(&CLASS, 44_100.0);
                inst.set_param(p.id, v);
                inst.set_param(RESO, 1.0);
                for n in [24, 60, 108] {
                    inst.note_on(n, 127);
                }
                let (l, r) = render(&mut inst, 8192);
                assert!(l.iter().chain(&r).all(|x| x.is_finite() && x.abs() < 8.0), "{} = {v}", p.name);
            }
        }
    }

    #[test]
    fn state_round_trips() {
        let mut a = Instance::new(&CLASS, 48_000.0);
        a.set_param(CUTOFF, 0.25);
        a.set_param(OSC2_SEMI, 31.0 / 48.0);
        a.set_param(MODE, 1.0);
        let saved = a.save();
        let mut b = Instance::new(&CLASS, 48_000.0);
        b.load(&saved).unwrap();
        assert_eq!(b.param_value(CUTOFF), 0.25);
        assert_eq!(b.param_text(OSC2_SEMI, b.param_value(OSC2_SEMI)), "+7 st");
        assert_eq!(b.param_text(MODE, b.param_value(MODE)), "Mono");
        assert_eq!(b.save(), saved);
        // A state from an engine with fewer parameters: the rest load at defaults.
        let mut old = saved.clone();
        old[8..12].copy_from_slice(&1u32.to_le_bytes());
        old.truncate(12 + 12);
        old.extend(0u32.to_le_bytes());
        b.load(&old).unwrap();
        assert_eq!(b.param_value(CUTOFF), 0.6);
        assert!(b.load(b"nope").is_err());
    }

    #[test]
    fn mono_legato_returns_to_held_note() {
        let mut r = Replicant::new(48_000.0);
        r.set_param(MODE, 1.0);
        let (mut l, mut rr) = (vec![0.0; 512], vec![0.0; 512]);
        r.note_on(48, 100);
        r.note_on(55, 100);
        r.process(&mut l, &mut rr, &Transport::default());
        assert_eq!(r.voices[0].note, 55);
        r.note_off(55);
        r.process(&mut l, &mut rr, &Transport::default());
        assert_eq!(r.voices[0].note, 48, "back to the note still held");
        assert!(r.voices[0].gate);
        assert!(r.voices[1..].iter().all(|v| !v.sounding()), "one voice");
        r.note_off(48);
        assert!(!r.voices[0].gate);
    }
}
