//! Maschine's Sampler, as docs/sampler-dsp.md describes it: per sound, voices that
//! stream a zone's sample (play range, loop with crossfade, zone fades, reverse),
//! resample it with an 8-point interpolator, then run Comp, Drive, Lo-fi, the filter
//! (LP2 / BP2 / HP2 or the EQ band), the amp envelope and the zone's gain and pan.
//! A modulation envelope, an LFO, velocity and the mod wheel modulate pitch, start,
//! cutoff, drive and pan. Section numbers below refer to the spec.
//!
//! A sound whose instrument is one of Maschine's synths (drum synths, Bass Synth, Poly
//! Synth) plays through the same [`SamplerRt`], holding the [`crate::synth::Synth`]
//! instead of zones: it is rendered, kept across edits and rendered ahead the same way.

use crate::sample::Sample;
use crate::sampler_tables as t;
use crate::synth::Synth;
use deckard_model::MidiKind;
use std::sync::Arc;

/// Control rate: modulation, filter coefficients and smoothing are updated every
/// 32 frames (§8.1).
const CHUNK: u32 = 32;
/// Voices kept per Sampler at most, whatever the polyphony (a safety limit).
const MAX_VOICES: usize = 128;
/// Below this an envelope counts as silent (§5.1).
const SILENT: f32 = 1e-4;
/// Frames a synth renders at a time.
const SYNTH_CHUNK: usize = 512;

/// `sq(x) = |x|·x`, the shape of almost every modulation amount.
fn sq(x: f32) -> f32 {
    x.abs() * x
}

/// Linear interpolation in `table` at fractional index `x` (0..len−1).
fn at(table: &[f32], x: f32) -> f32 {
    let x = x.clamp(0.0, (table.len() - 1) as f32);
    let i = x as usize;
    if i + 1 < table.len() { table[i] + (x - i as f32) * (table[i + 1] - table[i]) } else { table[i] }
}

/// Envelope segment times (§3), in frames.
fn attack_frames(v: f32, sr: f64) -> u64 {
    ms_to_frames(((t::lookup(&t::ATTACK_CURVE, v) * 8.922792) as f64 + std::f64::consts::LN_2).exp() - 2.0, sr)
}

fn hold_frames(v: f32, sr: f64) -> u64 {
    ms_to_frames(((t::lookup(&t::HOLD_CURVE, v) * 8.922792) as f64 + std::f64::consts::LN_2).exp() - 2.0, sr)
}

fn decay_frames(v: f32, sr: f64) -> u64 {
    ms_to_frames(((t::lookup(&t::DECAY_RELEASE_CURVE, v) * 9.433563) as f64 + std::f64::consts::LN_2).exp() - 2.0, sr)
}

fn ms_to_frames(ms: f64, sr: f64) -> u64 {
    (sr * ms.max(0.0) * 0.001) as u64
}

/// Envelope knob values in milliseconds, for display: (attack, hold, decay/release).
pub fn envelope_ms(attack: f64, hold: f64, decay: f64) -> (f64, f64, f64) {
    let ms = |c: &[f32; 128], v: f64, k: f32| ((t::lookup(c, v as f32) * k) as f64 + std::f64::consts::LN_2).exp() - 2.0;
    (ms(&t::ATTACK_CURVE, attack, 8.922792), ms(&t::HOLD_CURVE, hold, 8.922792), ms(&t::DECAY_RELEASE_CURVE, decay, 9.433563))
}

/// Cutoff knob (0..1, with modulation) to a fraction of Nyquist at 44.1 kHz (§8.5).
fn cutoff_fraction(c: f32) -> f32 {
    let x = c.clamp(0.0, 1.0) * 6.15;
    let e = x.exp() - 1.0;
    (e / 388.67682).clamp(0.0, 1.0) + 0.002
}

/// The cutoff a knob value shows, in Hz, for filter mode 1 (LP2), 2 (BP2) or 3 (HP2).
pub fn cutoff_hz(knob: f64, mode: i64) -> f64 {
    let f = cutoff_fraction(t::lookup(&t::CUTOFF_CURVE, knob as f32)).min(1.0) as f64;
    let k = match mode {
        2 => 0.82,
        3 => 0.92 * 0.92,
        _ => 0.99,
    };
    f * 22050.0 * k
}

/// The Lo-fi sample rate a knob value shows: `None` when off (§8.4).
pub fn lofi_rate_hz(knob: f64, sr: f64) -> Option<f64> {
    let r = t::lookup(&t::LOFI_RATE_CURVE, knob as f32) as f64;
    (r < 0.99999).then(|| (r * sr).max(50.0))
}

/// The Lo-fi bit depth a knob value gives: `None` when off.
pub fn lofi_bits(knob: f64) -> Option<f64> {
    let b = t::lookup(&t::LOFI_BITS_CURVE, knob as f32) as f64;
    (b < 0.99999).then(|| (16.0 * b).max(1.0))
}

/// The EQ band's centre frequency in Hz for a knob value (§8.5).
pub fn eq_hz(knob: f64) -> f64 {
    20.0 * 1000f64.powf(knob.clamp(0.0, 1.0))
}

/// The LFO's free-running rate in Hz, or its note division as (num, den) when synced
/// (§4).
pub fn lfo_rate(speed: f64) -> (f64, (i32, i32)) {
    let d = &t::LFO_RATE_DIVISIONS;
    let n = d.len();
    let x = speed.clamp(0.0, 1.0) * (n - 1) as f64;
    let i = ((x - 0.5).round() as i64).clamp(0, n as i64 - 2) as usize;
    let f = x - i as f64;
    let len = f * d[i + 1].0 as f64 / d[i + 1].1 as f64 + (1.0 - f) * d[i].0 as f64 / d[i].1 as f64;
    let j = ((x).round() as i64).clamp(0, n as i64 - 1) as usize;
    (1.0 / (2.0 * len), d[j])
}

/// One stage of [`Env`]: from level `s` to `e` along Maschine's exponential curve
/// with curvature `c` (§5.2).
#[derive(Clone, Copy, Default)]
struct Segment {
    x: f64,
    m: f64,
    d: f64,
    off: f64,
    left: u64,
}

impl Segment {
    fn new(s: f32, e: f32, frames: u64, curvature: f64) -> Segment {
        const L: f64 = -9.903487552536127;
        const H: f64 = 3.2188758248682006;
        let n = frames.max(1) as f64;
        let c = (L + (H - L) * (1.0 - curvature.abs())).exp();
        let ratio = if curvature > 0.0 { c / (c + 1.0) } else { (c + 1.0) / c };
        let x0 = if curvature > 0.0 { c + 1.0 } else { c };
        let d = if curvature > 0.0 { -(e - s) as f64 } else { (e - s) as f64 };
        Segment { x: x0, m: ratio.powf(1.0 / n), d, off: s as f64 - d * x0, left: frames }
    }

    fn next(&mut self) -> f32 {
        let y = self.off + self.d * self.x;
        self.x *= self.m;
        self.left -= 1;
        y as f32
    }
}

const C_ATTACK: f64 = -0.332;
const C_DECAY: f64 = 0.44269788; // 0x3ee2a84c

/// Envelope segment times and sustain level, for one envelope (§3, §5.1).
#[derive(Clone, Copy, Default, PartialEq)]
struct EnvTimes {
    attack: u64,
    hold: u64,
    decay: u64,
    release: u64,
    sustain: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    Done,
}

/// An AHDSR envelope (§5). `gated`: sustains until note-off, then releases (ADSR);
/// otherwise it runs attack, hold and decay to the sustain level and stops there.
#[derive(Clone, Copy)]
struct Env {
    times: EnvTimes,
    gated: bool,
    stage: Stage,
    seg: Segment,
    level: f32,
}

impl Env {
    fn new(times: EnvTimes, gated: bool) -> Env {
        let mut e = Env { times, gated, stage: Stage::Attack, seg: Segment::default(), level: 0.0 };
        e.enter(Stage::Attack);
        e
    }

    fn enter(&mut self, stage: Stage) {
        self.stage = stage;
        let t = self.times;
        match stage {
            Stage::Attack if t.attack > 0 => self.seg = Segment::new(0.0, 1.0, t.attack, C_ATTACK),
            Stage::Attack => {
                self.level = 1.0;
                self.enter(Stage::Hold)
            }
            Stage::Hold if t.hold > 0 => self.seg = Segment::new(1.0, 1.0, t.hold, 0.0),
            Stage::Hold => self.enter(Stage::Decay),
            Stage::Decay if t.decay > 0 => self.seg = Segment::new(self.level, t.sustain, t.decay, C_DECAY),
            Stage::Decay => {
                self.level = t.sustain;
                self.enter(Stage::Sustain)
            }
            Stage::Sustain if !self.gated => self.stage = Stage::Done,
            Stage::Sustain => {}
            Stage::Release if t.release > 0 => self.seg = Segment::new(self.level, 0.0, t.release, C_DECAY),
            Stage::Release => {
                self.level = 0.0;
                self.stage = Stage::Done
            }
            Stage::Done => {}
        }
    }

    /// Note-off: release from the current level (gated envelopes only).
    fn release(&mut self) {
        if self.gated && self.stage != Stage::Done && self.stage != Stage::Release {
            self.enter(Stage::Release);
        }
    }

    fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Attack | Stage::Hold | Stage::Decay | Stage::Release => {
                self.level = self.seg.next();
                if self.seg.left == 0 {
                    let next = match self.stage {
                        Stage::Attack => Stage::Hold,
                        Stage::Hold => Stage::Decay,
                        Stage::Decay => Stage::Sustain,
                        _ => Stage::Done,
                    };
                    if self.stage == Stage::Release {
                        self.level = 0.0;
                    }
                    self.enter(next);
                }
            }
            Stage::Sustain | Stage::Done => {}
        }
        self.level
    }

    fn done(&self) -> bool {
        self.stage == Stage::Done && self.level < SILENT
    }
}

/// A zone ready to play.
#[derive(Clone)]
pub struct ZoneRt {
    pub sample: Arc<Sample>,
    pub root: i64,
    pub keys: (i64, i64),
    pub vels: (i64, i64),
    /// Zone gain as stored (0.75 = unity) and pan (−1..1).
    pub gain: f32,
    pub pan: f32,
    /// Semitones.
    pub tune: f32,
    /// Play range and loop range, frames [start, end).
    pub play: (usize, usize),
    pub looped: Option<(usize, usize)>,
    pub x_fade: usize,
    /// Zone fade-in and fade-out, frames (§6.5).
    pub fade_in: usize,
    pub fade_out: usize,
}

/// The Sampler's fields modulation can drive ([`Settings::set_field`]).
pub const MODULATED: [&str; 41] = [
    "glide",
    "pitchbend",
    "tune",
    "start",
    "comp",
    "drive",
    "bits",
    "sample_rate",
    "cutoff",
    "resonance",
    "eq_frequency",
    "eq_width",
    "eq_gain",
    "velocity_start",
    "velocity_decay",
    "velocity_cutoff",
    "velocity_volume",
    "modwheel_start",
    "modwheel_cutoff",
    "modwheel_lfo",
    "modwheel_pan",
    "volume_envelope/attack",
    "volume_envelope/hold",
    "volume_envelope/decay",
    "volume_envelope/release",
    "volume_envelope/sustain",
    "mod_envelope/attack",
    "mod_envelope/hold",
    "mod_envelope/decay",
    "mod_envelope/release",
    "mod_envelope/sustain",
    "mod_envelope/pitch",
    "mod_envelope/cutoff",
    "mod_envelope/drive",
    "mod_envelope/pan",
    "lfo/speed",
    "lfo/phase",
    "lfo/pitch_amount",
    "lfo/cutoff_amount",
    "lfo/drive_amount",
    "lfo/pan_amount",
];

/// The Sampler's settings, converted from the stored values (§1).
#[derive(Clone, PartialEq)]
pub struct Settings {
    polyphony: usize,
    glide: f32,
    bend_range: f32,
    mode: usize,
    env_type: i64,
    reverse: bool,
    tune: f32,
    start: f32,
    comp: f32,
    drive: f32,
    lofi_bits: f32,
    lofi_rate: f32,
    filter: i64,
    cutoff: f32,
    resonance: f32,
    eq: (f32, f32, f32),
    vel_start: f32,
    vel_decay: f32,
    vel_cutoff: f32,
    vel_volume: f32,
    mw_start: f32,
    mw_cutoff: f32,
    mw_lfo: f32,
    mw_pan: f32,
    amp: EnvTimes,
    amp_decay_knob: f32,
    modenv: EnvTimes,
    env_pitch: f32,
    env_cutoff: f32,
    env_drive: f32,
    env_pan: f32,
    lfo_type: i64,
    lfo_speed: f32,
    lfo_phase: f32,
    lfo_sync: i64,
    lfo_pitch: f32,
    lfo_cutoff: f32,
    lfo_drive: f32,
    lfo_pan: f32,
    /// A synth's parameters (stored values, by field), when the sound plays one.
    synth: Vec<(String, f64)>,
}

impl Settings {
    /// Settings of a sound played by a synth: its module's parameters.
    pub fn for_synth(params: &[(String, f64)], sr: f64) -> Settings {
        Settings { synth: params.to_vec(), ..Settings::new(&deckard_model::Sampler::default(), sr) }
    }

    pub fn new(s: &deckard_model::Sampler, sr: f64) -> Settings {
        let g = |k: &str| s.get(k) as f32;
        let env = |e: &str| EnvTimes {
            attack: attack_frames(g(&format!("{e}/attack")), sr),
            hold: hold_frames(g(&format!("{e}/hold")), sr),
            decay: decay_frames(g(&format!("{e}/decay")), sr),
            release: decay_frames(g(&format!("{e}/release")), sr),
            sustain: g(&format!("{e}/sustain")).clamp(0.0, 1.0).powi(3),
        };
        Settings {
            polyphony: g("polyphony").max(0.0) as usize,
            glide: g("glide"),
            bend_range: sq(g("pitchbend")),
            mode: g("mode").max(0.0) as usize,
            env_type: g("envelope_type") as i64,
            reverse: g("reverse") != 0.0,
            tune: g("tune") * 3.0,
            start: sq(g("start")),
            comp: g("comp"),
            drive: g("drive"),
            lofi_bits: g("bits"),
            lofi_rate: g("sample_rate"),
            filter: g("filter") as i64,
            cutoff: g("cutoff"),
            resonance: g("resonance"),
            eq: (g("eq_frequency"), g("eq_width"), g("eq_gain")),
            vel_start: sq(g("velocity_start")),
            vel_decay: sq(g("velocity_decay")),
            vel_cutoff: sq(g("velocity_cutoff")),
            vel_volume: sq(g("velocity_volume")),
            mw_start: sq(g("modwheel_start")),
            mw_cutoff: sq(g("modwheel_cutoff")),
            mw_lfo: sq(g("modwheel_lfo")),
            mw_pan: sq(g("modwheel_pan")),
            amp: env("volume_envelope"),
            amp_decay_knob: g("volume_envelope/decay"),
            modenv: env("mod_envelope"),
            env_pitch: sq(g("mod_envelope/pitch")),
            env_cutoff: sq(g("mod_envelope/cutoff")),
            env_drive: sq(g("mod_envelope/drive")),
            env_pan: sq(g("mod_envelope/pan")),
            lfo_type: g("lfo/type") as i64,
            lfo_speed: g("lfo/speed"),
            lfo_phase: g("lfo/phase"),
            lfo_sync: g("lfo/sync_mode") as i64,
            lfo_pitch: sq(g("lfo/pitch_amount")),
            lfo_cutoff: sq(g("lfo/cutoff_amount")),
            lfo_drive: sq(g("lfo/drive_amount")),
            lfo_pan: sq(g("lfo/pan_amount")),
            synth: Vec::new(),
        }
    }

    /// Set field `k` (as stored: `cutoff`, `volume_envelope/attack`, …) to stored value
    /// `v`, converted as [`Settings::new`] does: modulation, on the audio thread (no
    /// allocating). Returns whether `k` is one of the [`MODULATED`] fields.
    pub fn set_field(&mut self, k: &str, v: f64, sr: f64) -> bool {
        let x = v as f32;
        let env = |e: &mut EnvTimes, part: &str| match part {
            "attack" => e.attack = attack_frames(x, sr),
            "hold" => e.hold = hold_frames(x, sr),
            "decay" => e.decay = decay_frames(x, sr),
            "release" => e.release = decay_frames(x, sr),
            _ => e.sustain = x.clamp(0.0, 1.0).powi(3),
        };
        match k {
            "glide" => self.glide = x,
            "pitchbend" => self.bend_range = sq(x),
            "tune" => self.tune = x * 3.0,
            "start" => self.start = sq(x),
            "comp" => self.comp = x,
            "drive" => self.drive = x,
            "bits" => self.lofi_bits = x,
            "sample_rate" => self.lofi_rate = x,
            "cutoff" => self.cutoff = x,
            "resonance" => self.resonance = x,
            "eq_frequency" => self.eq.0 = x,
            "eq_width" => self.eq.1 = x,
            "eq_gain" => self.eq.2 = x,
            "velocity_start" => self.vel_start = sq(x),
            "velocity_decay" => self.vel_decay = sq(x),
            "velocity_cutoff" => self.vel_cutoff = sq(x),
            "velocity_volume" => self.vel_volume = sq(x),
            "modwheel_start" => self.mw_start = sq(x),
            "modwheel_cutoff" => self.mw_cutoff = sq(x),
            "modwheel_lfo" => self.mw_lfo = sq(x),
            "modwheel_pan" => self.mw_pan = sq(x),
            "mod_envelope/pitch" => self.env_pitch = sq(x),
            "mod_envelope/cutoff" => self.env_cutoff = sq(x),
            "mod_envelope/drive" => self.env_drive = sq(x),
            "mod_envelope/pan" => self.env_pan = sq(x),
            "lfo/speed" => self.lfo_speed = x,
            "lfo/phase" => self.lfo_phase = x,
            "lfo/pitch_amount" => self.lfo_pitch = sq(x),
            "lfo/cutoff_amount" => self.lfo_cutoff = sq(x),
            "lfo/drive_amount" => self.lfo_drive = sq(x),
            "lfo/pan_amount" => self.lfo_pan = sq(x),
            _ => match k.split_once('/') {
                Some(("volume_envelope", part @ ("attack" | "hold" | "decay" | "release" | "sustain"))) => {
                    if part == "decay" {
                        self.amp_decay_knob = x;
                    }
                    env(&mut self.amp, part);
                }
                Some(("mod_envelope", part @ ("attack" | "hold" | "decay" | "release" | "sustain"))) => env(&mut self.modenv, part),
                _ => return false,
            },
        }
        true
    }

    /// Notes that may sound at once (§9: Legato and "1" allow one).
    fn note_limit(&self) -> usize {
        match self.polyphony {
            0 | 1 => 1,
            i => t::POLYPHONY_LIMITS[(i - 2).min(t::POLYPHONY_LIMITS.len() - 1)],
        }
    }

    fn legato(&self) -> bool {
        self.polyphony == 0
    }
}

/// Where a voice reads its zone's sample: play range, loop and crossfade, zone fades
/// and direction (§6.4, §6.5). Yields source frames one at a time.
#[derive(Clone, Copy)]
struct Stream {
    pos: i64,
    forward: bool,
    play: (i64, i64),
    looped: Option<(i64, i64)>,
    x_fade: i64,
    fade_in: i64,
    fade_out: i64,
    done: bool,
}

impl Stream {
    fn next(&mut self, data: &[[f32; 2]]) -> [f32; 2] {
        if self.done {
            return [0.0; 2];
        }
        let (start, end) = self.play;
        let p = self.pos;
        if p < start || p >= end {
            self.done = true;
            return [0.0; 2];
        }
        let mut x = data[p as usize];
        // Loop crossfade: the tail before the loop end fades into the material before
        // the loop start (forward playback).
        if let Some((ls, le)) = self.looped
            && self.forward
        {
            let xf = self.x_fade.min(le - ls).min(ls - start);
            if xf > 0 && p >= le - xf && p < le {
                let w = (le - p) as f32 / xf as f32;
                let q = (ls - (le - p)) as usize;
                let y = data[q];
                x = [x[0] * w + y[0] * (1.0 - w), x[1] * w + y[1] * (1.0 - w)];
            }
        }
        // Zone fades, by position in the play range.
        let mut g = 1.0;
        if self.fade_in > 0 && p - start < self.fade_in {
            g *= (p - start) as f32 / self.fade_in as f32;
        }
        if self.fade_out > 0 && end - p <= self.fade_out {
            g *= (end - p - 1) as f32 / self.fade_out as f32;
        }
        if g != 1.0 {
            x = [x[0] * g, x[1] * g];
        }
        // Advance.
        if self.forward {
            self.pos += 1;
            if let Some((ls, le)) = self.looped
                && self.pos >= le
            {
                self.pos = ls;
            }
        } else {
            self.pos -= 1;
            if let Some((ls, le)) = self.looped
                && self.pos < ls
            {
                self.pos = le - 1;
            }
        }
        x
    }
}

/// The 8-point interpolator's tap weights for `t = 1 − frac` (§7), for taps
/// x[n−3] .. x[n+4].
fn weights(t: f32) -> [f32; 8] {
    let h = |c: [f32; 5], t: f32| ((((c[0] * t + c[1]) * t + c[2]) * t + c[3]) * t + c[4]) * t;
    [
        h([-0.020964433, 0.075699046, -0.061509572, 0.007359662, -0.00058470765], t),
        h([0.14338401, -0.38360944, 0.18016043, 0.03248506, 0.027579939], t),
        h([-0.3720349, 0.90449077, -0.08861077, -0.25935876, -0.18448636], t),
        h([0.6183112, -1.51671, -0.032761108, 1.1424075, 0.78875244], t),
        (((-0.6183112 * t + 1.574846) * t - 0.0835108) * t - 1.873024) * t * t + 1.0,
        h([0.3720349, -0.95568377, 0.1909967, 1.1814046, -0.78875244], t),
        h([-0.14338401, 0.3333106, -0.07956275, -0.2948502, 0.18448636], t),
        h([0.020964433, -0.02912311, -0.031642303, 0.06738092, -0.027579939], t),
    ]
}

/// A direct-form-I biquad (§8.5): y = b0x + b1x1 + b2x2 + a1y1 + a2y2 (feedback signs
/// included in a1, a2).
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: [f64; 2],
    x2: [f64; 2],
    y1: [f64; 2],
    y2: [f64; 2],
}

impl Biquad {
    fn set(&mut self, (b0, b1, b2, a1, a2): (f64, f64, f64, f64, f64)) {
        (self.b0, self.b1, self.b2, self.a1, self.a2) = (b0, b1, b2, a1, a2);
    }

    /// One frame. Computed in double precision: near the lowest cutoffs the poles sit
    /// almost on the unit circle, where single precision drifts and can run away while
    /// the coefficients move.
    fn run(&mut self, x: [f32; 2]) -> [f32; 2] {
        let mut y = [0.0; 2];
        for c in 0..2 {
            let xc = x[c] as f64;
            let yc = self.b0 * xc + self.b1 * self.x1[c] + self.b2 * self.x2[c] + self.a1 * self.y1[c] + self.a2 * self.y2[c] + 1e-28;
            self.x2[c] = self.x1[c];
            self.x1[c] = xc;
            self.y2[c] = self.y1[c];
            self.y1[c] = yc;
            y[c] = yc as f32;
        }
        y
    }

    fn reset(&mut self) {
        (self.x1, self.x2, self.y1, self.y2) = ([0.0; 2], [0.0; 2], [0.0; 2], [0.0; 2]);
    }
}

/// LP2 / HP2 coefficients from the Butterworth prototype (§8.5).
fn two_pole(f: f32, q: f32, high: bool) -> (f64, f64, f64, f64, f64) {
    let [pb1, pb2, pa1, pa2, pb0] = t::FILTER_PROTOTYPE.map(|x| x as f64);
    let (f, q) = (f as f64, q as f64);
    let k = if high {
        let w = f * std::f64::consts::FRAC_PI_2 * 0.92 * 0.9971332;
        -(0.5 + w).cos() / (0.5 - w).cos()
    } else {
        let w = f * std::f64::consts::FRAC_PI_2 * 0.99 * 0.9987809;
        (0.5 - w).sin() / (0.5 + w).sin()
    };
    let a1p = pa1;
    let a2p = pa2 * (1.0 + 2.687 * q);
    let d = 1.0 / ((k * a1p + 1.0) - k * k * a2p);
    let mut a1 = (a1p + 2.0 * k + k * k * a1p - 2.0 * k * a2p) * d;
    let a2 = (a2p - a1p * k - k * k) * d;
    let b0 = (pb0 - k * pb1 + k * k * pb2) * d;
    let mut b1 = (pb1 - 2.0 * k * pb0 + k * k * pb1 - 2.0 * k * pb2) * d;
    let b2 = (pb2 - k * pb1 + k * k * pb0) * d;
    let comp = (1.0 - q) * 0.55 + 0.45;
    let g = if high {
        a1 = -a1;
        b1 = -b1;
        comp * ((a1 - a2) + 1.0) / (b0 - b1 + b2)
    } else {
        comp * (1.0 - (a1 + a2)) / (b0 + b1 + b2)
    };
    (g * b0, g * b1, g * b2, a1, a2)
}

/// BP2: two first-order sections, (b0, b1, a1) each (§8.5).
fn band_pass(f: f32) -> [(f64, f64, f64); 2] {
    let [b1p, a1p, b0p] = t::BANDPASS_PROTOTYPE.map(|x| x as f64);
    let w = f as f64 * std::f64::consts::FRAC_PI_2 * 0.82;
    let k1 = (0.5 - w * 0.9987809).sin() / (0.5 + w * 0.9987809).sin();
    let d1 = 1.0 / (k1 * a1p + 1.0);
    let (b0, b1, a) = (d1 * (b0p - k1 * b1p), d1 * (b1p - k1 * b0p), d1 * (a1p + k1));
    let g1 = (1.0 - a) / (b0 + b1);
    let k2 = -0.99 * (0.5 + w * 0.9971332).cos() / (0.5 - w * 0.9971332).cos();
    let d2 = 1.0 / (k2 * a1p + 1.0);
    let (c0, c1, a2) = (d2 * (b0p - k2 * b1p), d2 * (b1p - k2 * b0p), d2 * (k2 + a1p));
    let g2 = (1.0 - a2) / (c0 + c1);
    [(g1 * b0, g1 * b1, a), (g2 * c0, -g2 * c1, -a2)]
}

/// EQ peaking band (§8.5): (b0, b1, b2, a1, a2) with feedback signs.
fn peak_eq(freq: f32, width: f32, gain: f32, sr: f64) -> (f64, f64, f64, f64, f64) {
    let hz = eq_hz(freq as f64);
    let w0 = (hz / sr).min(0.49) * std::f64::consts::TAU;
    let bw = 2f64.powf(width as f64 * 8.0 / 3.0 + 1.0 / 3.0);
    let alpha = w0.sin() * (bw - 1.0 / bw) / 2.0;
    let a = 2f64.powf(gain as f64 * 18.0 / 6.0);
    let a0 = 1.0 + alpha / a;
    let c = w0.cos();
    (((1.0 + alpha * a) / a0), (-2.0 * c / a0), ((1.0 - alpha * a) / a0), (2.0 * c / a0), ((alpha / a - 1.0) / a0))
}

/// The filter's response for its display: `n` points (x 0..1, gain in dB) at 44.1 kHz.
/// x is the knob position whose cutoff (or EQ frequency) is there, so a handle at the
/// knob's value sits on the corner. Mode as the `filter` parameter (0 off, 1 LP2, 2 BP2,
/// 3 HP2, 4 EQ).
pub fn filter_response(mode: i64, cutoff: f64, resonance: f64, eq: (f64, f64, f64), n: usize) -> Vec<(f32, f32)> {
    let sr = 44_100.0;
    let q = (1.0 - (1.0 - resonance) * (1.0 - resonance)).clamp(0.0, 1.0) as f32;
    let fr = cutoff_fraction(t::lookup(&t::CUTOFF_CURVE, cutoff.clamp(0.0, 1.0) as f32)).min(1.0) * 0.998004;
    // Sections in the engine's form (feedback signs).
    let sections: Vec<(f64, f64, f64, f64, f64)> = match mode {
        1 => vec![two_pole(fr, q, false)],
        3 => vec![two_pole(fr, q, true)],
        2 => band_pass(fr).iter().map(|&(b0, b1, a)| (b0, b1, 0.0, a, 0.0)).collect(),
        4 => vec![peak_eq(eq.0 as f32, eq.1 as f32, eq.2 as f32, sr)],
        _ => Vec::new(),
    };
    (0..n)
        .map(|i| {
            let x = i as f64 / (n - 1).max(1) as f64;
            let hz = if mode == 4 { eq_hz(x) } else { cutoff_hz(x, mode.clamp(1, 3)) };
            let w = std::f64::consts::TAU * (hz / sr).min(0.4999);
            let (c1, s1, c2, s2) = (w.cos(), -w.sin(), (2.0 * w).cos(), -(2.0 * w).sin());
            let mag: f64 = sections
                .iter()
                .map(|&(b0, b1, b2, a1, a2)| {
                    let num = ((b0 + b1 * c1 + b2 * c2).powi(2) + (b1 * s1 + b2 * s2).powi(2)).sqrt();
                    let den = ((1.0 - a1 * c1 - a2 * c2).powi(2) + (a1 * s1 + a2 * s2).powi(2)).sqrt();
                    num / den.max(1e-12)
                })
                .product();
            (x as f32, (20.0 * mag.max(1e-6).log10()) as f32)
        })
        .collect()
}

/// One-pole coefficient reaching 99 % in `ms` (§8.2).
fn smoothing(ms: f64, sr: f64) -> f32 {
    let n = (sr * ms * 0.001 - 1.0).max(1.0);
    (1.0 - 2f64.powf(-(100f64.log2()) / n)) as f32
}

#[derive(Clone, Copy)]
struct Coeffs {
    c50: f32,
    gain: f32,
    release: f32,
}

/// Comp (ShaperDrum, §8.2).
#[derive(Clone, Copy, Default)]
struct Comp {
    cur: f32,
    env: f32,
    g: f32,
}

impl Comp {
    fn run(&mut self, x: [f32; 2], target: f32, c: Coeffs) -> [f32; 2] {
        let step = (target - self.cur) * c.c50;
        self.cur = if step.abs() < 1e-4 { target } else { self.cur + step };
        if self.cur <= 0.0001 * 50.0 && target <= 0.0001 * 50.0 {
            return x;
        }
        let peak = x[0].abs().max(x[1].abs());
        self.env = if peak >= self.env { peak } else { self.env + c.release * (peak - self.env) + 1e-12 };
        let lev = (self.env + 1e-5).min(1.0);
        let row = self.cur.clamp(0.0, 50.0);
        let r0 = (row as usize).min(51);
        let r1 = (r0 + 1).min(51);
        let col = lev * 127.0;
        let curve = |r: usize| at(&t::COMP_CURVES[r * 128..r * 128 + 128], col);
        let fr = row - r0 as f32;
        let v = curve(r0) + fr * (curve(r1) - curve(r0));
        let target_gain = v / lev;
        self.g += c.gain * (target_gain - self.g) + 1e-12;
        let k = self.g * at(&t::COMP_GAIN_CORRECTION, self.cur);
        [x[0] * k, x[1] * k]
    }
}

/// Drive (ShaperEnhanced, §8.3).
#[derive(Clone, Copy, Default)]
struct Drive {
    cur: f32,
}

impl Drive {
    fn run(&mut self, x: [f32; 2], target: f32, c50: f32) -> [f32; 2] {
        let step = (target - self.cur) * c50;
        self.cur = if step.abs() < 1e-4 { target } else { self.cur + step };
        if self.cur <= 0.0001 * 127.0 {
            return x;
        }
        let amount = at(&t::DRIVE_AMOUNT, self.cur);
        let mix = at(&t::DRIVE_MIX, self.cur);
        let corr = at(&t::DRIVE_GAIN_CORRECTION, self.cur);
        let f = |x: f32| {
            let u = (x * amount).clamp(-8.0, 8.0);
            x + mix * (corr * u * (1.0 - u.abs() / 16.0) - x)
        };
        [f(x[0]), f(x[1])]
    }
}

/// Lo-fi: bit reduction then sample-rate hold (§8.4).
#[derive(Clone, Copy, Default)]
struct Lofi {
    count: f32,
    held: [f32; 2],
    primed: bool,
}

impl Lofi {
    fn run(&mut self, x: [f32; 2], levels: Option<f32>, hold: Option<f32>) -> [f32; 2] {
        let mut y = x;
        if let Some(l) = levels {
            y = [(y[0] * l).trunc() / l, (y[1] * l).trunc() / l];
        }
        if let Some(h) = hold {
            if !self.primed {
                self.primed = true;
                self.count = 0.0;
                self.held = y;
            } else if self.count >= h {
                self.count -= h;
                self.held = y;
            } else {
                self.count += 1.0;
            }
            y = self.held;
        }
        y
    }
}

/// Voices a Sampler has room for from the start (more grow the list).
const VOICE_ROOM: usize = 24;

#[derive(Clone, Copy)]
struct Voice {
    id: u64,
    note: i64,
    zone: usize,
    vel: f32,
    age: u64,
    stream: Stream,
    /// x[n−3] .. x[n+4] and the 16.16 fraction (§7).
    hist: [[f32; 2]; 8],
    frac: u32,
    /// Frames shifted in after the stream ended (the voice ends when the tail is out).
    tail: u32,
    amp: Env,
    amp_on: bool,
    modenv: Env,
    lfo_phase: f64,
    lfo_inc: f64,
    lfo_value: f32,
    lfo_seed: u32,
    /// Pitch in octaves, without glide and modulation.
    octaves: f32,
    rate_ratio: f64,
    inc: u32,
    /// Static gain (zone gain, velocity → volume) and zone pan, per channel.
    gain: [f32; 2],
    pan_mod: f32,
    ctl_left: u32,
    // Effects.
    comp: Comp,
    drive: Drive,
    drive_target: f32,
    lofi: Lofi,
    filter: [Biquad; 2],
    /// The filter mode the filter state belongs to.
    filter_mode: i64,
    /// Pan modulation target (set each control step; `pan_mod` follows it).
    pan_target: f32,
    cutoff: f32,
    cutoff_set: bool,
    eq: Biquad,
    /// Linear fade-out (stolen, CC 120, stop): (frames left, total).
    fade: Option<(u32, u32)>,
    released: bool,
    sustained: bool,
}

/// Modulation and transport state shared by the voices of one Sampler.
pub struct Context {
    pub tempo: f64,
    /// Song position in beats (quarter notes), for the LFO's Lock mode.
    pub beats: f64,
}

/// One sound's Sampler: settings, zones, voices and the controllers.
pub struct SamplerRt {
    pub settings: Settings,
    pub zones: Vec<ZoneRt>,
    /// Sound and group level/pan combined.
    pub mix: (f32, f32),
    sr: f64,
    voices: Vec<Voice>,
    clock: u64,
    coeffs: Coeffs,
    mod_wheel: f32,
    bend: f32,
    pedal: bool,
    /// Keys held, in order (legato and glide).
    held: Vec<i64>,
    glide: f32,
    glide_target: f32,
    glide_step: f32,
    first_note: i64,
    /// Free-running LFO phase (cycles).
    lfo_phase: f64,
    seed: u32,
    last_peak: f32,
    /// The synth playing instead of zones: its class, and room to render it.
    synth: Option<(&'static str, Box<dyn Synth>, Vec<f32>, Vec<f32>)>,
    /// Takes the synth to new settings (knob moves).
    synth_glide: Option<crate::glide::Glide>,
    /// The Audio plug-in's loop playing instead of zones.
    audio: Option<Box<crate::audio_loop::LoopRt>>,
}

impl SamplerRt {
    pub fn new(settings: Settings, zones: Vec<ZoneRt>, mix: (f32, f32), sr: f64) -> SamplerRt {
        let zones_len = zones.len();
        SamplerRt {
            settings,
            zones,
            mix,
            sr,
            // Room for the voices a sound usually plays, made here (on the UI thread):
            // growing it while notes start would allocate on the audio thread.
            voices: Vec::with_capacity(if zones_len == 0 { 0 } else { VOICE_ROOM }),
            clock: 0,
            coeffs: Coeffs { c50: smoothing(50.0, sr), gain: smoothing(2.67874, sr), release: smoothing(25.4764, sr) },
            mod_wheel: 0.0,
            bend: 0.0,
            pedal: false,
            held: Vec::with_capacity(16),
            glide: 0.0,
            glide_target: 0.0,
            glide_step: 0.0,
            first_note: 0,
            lfo_phase: 0.0,
            seed: 0x1234_5678,
            last_peak: 0.0,
            synth: None,
            synth_glide: None,
            audio: None,
        }
    }

    /// A sound played by the Audio plug-in (its loop, kept in time with the song).
    pub fn audio(audio: crate::audio_loop::LoopRt, sr: f64) -> SamplerRt {
        let mut s = SamplerRt::new(Settings::for_synth(&[], sr), Vec::new(), (1.0, 1.0), sr);
        s.audio = Some(Box::new(audio));
        s
    }

    /// Where the song is (beats) and its tempo, at the start of the next render (the
    /// Audio plug-in follows it).
    pub fn set_song(&mut self, beats: f64, tempo: f64) {
        if let Some(a) = self.audio.as_mut() {
            a.set_song(beats, tempo);
        }
    }

    /// The Audio plug-in plays take `take` from song beat `origin` on, or stops.
    pub fn cue(&mut self, cue: Option<(usize, f64)>) {
        if let Some(a) = self.audio.as_mut() {
            a.cue(cue);
        }
    }

    /// The Audio plug-in's take playing, if any.
    pub fn cued(&self) -> Option<usize> {
        self.audio.as_ref().and_then(|a| a.cued())
    }

    /// A sound played by the synth of module `class` (see [`crate::synth::make`]), if
    /// DECKARD has it.
    pub fn synth(class: &str, params: &[(String, f64)], mix: (f32, f32), sr: f64) -> Option<SamplerRt> {
        let class = crate::synth::CLASSES.iter().copied().find(|c| *c == class)?;
        let synth = crate::synth::make(class, params, sr)?;
        let mut s = SamplerRt::new(Settings::for_synth(params, sr), Vec::new(), mix, sr);
        s.synth = Some((class, synth, vec![0.0; SYNTH_CHUNK], vec![0.0; SYNTH_CHUNK]));
        s.synth_glide = Some(crate::glide::Glide::new(&s.settings.synth, sr));
        Some(s)
    }

    /// Modulation (the audio thread): field `k` plays at stored value `v` until the next
    /// edit or modulation: a Sampler field ([`MODULATED`]), or a parameter of the synth
    /// playing (one it has; it glides there).
    pub fn modulate(&mut self, k: &str, v: f64) {
        let Some((_, synth, ..)) = self.synth.as_mut() else {
            self.settings.set_field(k, v, self.sr);
            return;
        };
        let Some(x) = self.settings.synth.iter_mut().find(|(f, _)| f == k) else { return };
        if x.1 == v {
            return;
        }
        x.1 = v;
        let params = &self.settings.synth;
        match self.synth_glide.as_mut() {
            Some(g) => synth.set(if g.retarget(params) { g.now() } else { params }),
            None => synth.set(params),
        }
    }

    #[cfg(test)]
    pub(crate) fn settings_cutoff(&self) -> f32 {
        self.settings.cutoff
    }

    #[cfg(test)]
    pub(crate) fn synth_param(&self, k: &str) -> Option<f64> {
        self.settings.synth.iter().find(|(f, _)| f == k).map(|x| x.1)
    }

    /// Put in new settings (an edit), handing back the old ones in `st`.
    pub fn swap_settings(&mut self, st: &mut Settings) {
        std::mem::swap(&mut self.settings, st);
        if let Some((_, synth, ..)) = self.synth.as_mut() {
            let params = &self.settings.synth;
            match self.synth_glide.as_mut() {
                Some(g) => synth.set(if g.retarget(params) { g.now() } else { params }),
                None => synth.set(params),
            }
        }
    }

    /// Whether [`SamplerRt::render`] must run: while voices sound, and always for a
    /// synth (its control clock, LFOs and noise run on while silent, as in Maschine).
    pub fn runs(&self) -> bool {
        self.synth.is_some() || !self.voices.is_empty() || self.audio.as_ref().is_some_and(|a| a.runs())
    }

    pub fn voices(&self) -> usize {
        match &self.synth {
            Some((_, s, ..)) => s.voices(),
            None => self.voices.len(),
        }
    }

    /// Where the newest voice playing zone `zone` is in its sample (frames), if any:
    /// the Sample Editor's playhead.
    pub fn play_position(&self, zone: usize) -> Option<usize> {
        self.voices
            .iter()
            .filter(|v| v.zone == zone && !v.stream.done && v.fade.is_none())
            .max_by_key(|v| v.age)
            .map(|v| v.stream.pos.max(0) as usize)
    }

    /// The peak level of the last [`SamplerRt::render`], before the engine's clamp.
    pub fn last_peak(&self) -> f32 {
        self.last_peak
    }

    /// Take over the voices and controller state of `old` (the same sound before a
    /// project change). Voices of zones that changed sample fade out.
    pub fn adopt(&mut self, mut old: SamplerRt) {
        if let (Some(new), Some(o)) = (self.audio.as_mut(), old.audio.take()) {
            new.adopt(*o);
            return;
        }
        // The same synth keeps playing, with the new settings.
        if let (Some(new), Some(o)) = (self.synth.as_mut(), old.synth.take()) {
            if new.0 == o.0 {
                *new = o;
                new.1.set(&self.settings.synth);
            }
            return;
        }
        let same = |a: &ZoneRt, b: &ZoneRt| Arc::ptr_eq(&a.sample, &b.sample);
        for v in old.voices.drain(..) {
            match (old.zones.get(v.zone), self.zones.get(v.zone)) {
                (Some(a), Some(b)) if same(a, b) => {}
                _ => continue,
            }
            self.voices.push(v);
        }
        self.clock = old.clock;
        (self.mod_wheel, self.bend, self.pedal) = (old.mod_wheel, old.bend, old.pedal);
        self.held = old.held;
        (self.glide, self.glide_target, self.glide_step, self.first_note) = (old.glide, old.glide_target, old.glide_step, old.first_note);
        self.lfo_phase = old.lfo_phase;
    }

    fn free_lfo_hz(&self) -> f64 {
        lfo_rate(self.settings.lfo_speed as f64).0
    }

    /// Note-on (§9). `id` identifies the note for its note-off.
    pub fn note_on(&mut self, id: u64, note: i64, velocity: i64, ctx: &Context) {
        if let Some(a) = self.audio.as_mut() {
            a.set_song(ctx.beats, ctx.tempo);
            a.note_on(note);
            return;
        }
        if let Some((_, s, ..)) = self.synth.as_mut() {
            s.set_tempo(ctx.tempo);
            s.note_on(note, if velocity <= 0 { 64 } else { velocity.min(127) });
            return;
        }
        let velocity = if velocity <= 0 { 64 } else { velocity.min(127) };
        let (legato, glide) = (self.settings.legato(), self.settings.glide);
        if legato {
            let had = !self.held.is_empty();
            self.held.retain(|&n| n != note);
            self.held.push(note);
            let frames = (glide * glide * self.sr as f32).round().max(1.0);
            if had {
                // The sounding voice glides to the new key.
                self.glide_target = (note - self.first_note) as f32 / 12.0;
                self.glide_step = (self.glide_target - self.glide) / frames;
                if self.voices.iter().any(|v| v.fade.is_none()) {
                    return;
                }
            }
            for v in &mut self.voices {
                v.fade_out(self.sr);
            }
            self.first_note = note;
            (self.glide, self.glide_target, self.glide_step) = (0.0, 0.0, 0.0);
        }
        self.clock += 1;
        let age = self.clock;
        let mut started = false;
        for zi in 0..self.zones.len() {
            let z = &self.zones[zi];
            if !(z.keys.0..=z.keys.1).contains(&note) || !(z.vels.0..=z.vels.1).contains(&velocity) || z.play.1 <= z.play.0 {
                continue;
            }
            let v = self.make_voice(id, note, velocity, zi, age, ctx);
            if self.voices.len() >= MAX_VOICES
                && let Some(i) = self.voices.iter().enumerate().min_by_key(|(_, v)| v.age).map(|(i, _)| i)
            {
                self.voices.swap_remove(i);
            }
            self.voices.push(v);
            started = true;
        }
        if !started {
            return;
        }
        // Voice limit, counted in notes (a note's zones share its age): fade out the
        // oldest beyond it. Without allocating (this runs on the audio thread).
        let limit = self.settings.note_limit();
        loop {
            let v = &self.voices;
            // Live voices whose age no earlier live voice has: one per note.
            let notes =
                (0..v.len()).filter(|&i| v[i].fade.is_none() && !v[..i].iter().any(|w| w.fade.is_none() && w.age == v[i].age)).count();
            if notes <= limit {
                break;
            }
            let Some(oldest) = self.voices.iter().filter(|v| v.fade.is_none()).map(|v| v.age).min() else { break };
            for v in self.voices.iter_mut().filter(|v| v.age == oldest) {
                v.fade_out(self.sr);
            }
        }
    }

    fn make_voice(&mut self, id: u64, note: i64, velocity: i64, zi: usize, age: u64, ctx: &Context) -> Voice {
        let s = self.settings.clone();
        let free_hz = self.free_lfo_hz();
        self.seed = self.seed.wrapping_mul(0x0BB3_8435).wrapping_add(0x3619_636B);
        let z = &self.zones[zi];
        let vel = velocity as f32 * 0.007874016;
        let mw = self.mod_wheel;
        // Amp envelope: velocity → decay re-derives the decay time (§3).
        let mut amp_t = s.amp;
        if s.vel_decay != 0.0 {
            let a = s.vel_decay;
            let tt = t::lookup(&t::VELOCITY_DECAY_CURVE, s.amp_decay_knob);
            let u = if a >= 0.0 { 1.0 - vel } else { vel };
            let ms = (((1.0 - a.abs() * u) * tt * 8.922792) as f64 + std::f64::consts::LN_2).exp() - 2.0;
            amp_t.decay = ms_to_frames(ms, self.sr);
        }
        let (amp_on, gated) = match s.env_type {
            0 => (false, false),
            1 => (true, false),
            _ => (true, true),
        };
        if s.env_type == 1 {
            amp_t.release = 0;
            amp_t.sustain = 0.0;
        } else if s.env_type == 2 {
            amp_t.hold = 0;
        }
        let mut mod_t = s.modenv;
        if s.env_type == 2 {
            mod_t.hold = 0;
        } else {
            mod_t.sustain = 0.0;
        }
        // Start (§6.2) and the stream.
        let len = (z.play.1 - z.play.0) as f32;
        let start = (s.start + vel * s.vel_start + mw * s.mw_start).clamp(0.0, 1.0);
        let offset = (start * len) as i64;
        let forward = !s.reverse;
        let looping = amp_on && z.looped.is_some();
        let stream = Stream {
            pos: if forward { z.play.0 as i64 + offset } else { z.play.1 as i64 - 1 - offset },
            forward,
            play: (z.play.0 as i64, z.play.1 as i64),
            looped: if looping { z.looped.map(|(a, b)| (a as i64, b as i64)) } else { None },
            x_fade: z.x_fade as i64,
            fade_in: z.fade_in as i64,
            fade_out: z.fade_out as i64,
            done: false,
        };
        // Gain and pan (§6.6).
        let mut g = 2.3948107 * z.gain.powi(5) + 0.767467 * z.gain * z.gain;
        let a = s.vel_volume;
        let u = if a >= 0.0 { 1.0 - vel } else { vel };
        g *= (1.0 - a.abs() * u).powi(3);
        let p = z.pan;
        // 0.70711 and 0.29289 in Maschine.
        const K: f32 = std::f32::consts::FRAC_1_SQRT_2;
        let gain = [(1.0 - K * p - (1.0 - K) * p * p) * g, (1.0 + K * p - (1.0 - K) * p * p) * g];
        // LFO (§4).
        let (lfo_hz, phase) = match s.lfo_sync {
            0 => (free_hz, self.lfo_phase + s.lfo_phase as f64),
            sync => {
                let (_, (num, den)) = lfo_rate(s.lfo_speed as f64);
                let ratio = den as f64 / num as f64 * 0.25;
                let hz = ctx.tempo / 60.0 * ratio;
                if sync == 1 { (hz, s.lfo_phase as f64) } else { (hz, s.lfo_phase as f64 + ratio * ctx.beats.rem_euclid(1.0 / ratio)) }
            }
        };
        let octaves = z.tune / 12.0 + s.tune + (note - z.root) as f32 / 12.0;
        let mut v = Voice {
            id,
            note,
            zone: zi,
            vel,
            age,
            stream,
            hist: [[0.0; 2]; 8],
            frac: 0,
            tail: 0,
            amp: Env::new(amp_t, gated),
            amp_on,
            modenv: Env::new(mod_t, gated),
            lfo_phase: phase.rem_euclid(1.0),
            lfo_inc: lfo_hz / self.sr,
            lfo_value: 0.0,
            lfo_seed: self.seed,
            octaves,
            rate_ratio: z.sample.rate as f64 / self.sr,
            inc: 65536,
            gain,
            pan_mod: 0.0,
            ctl_left: 0,
            comp: Comp { g: 1.0, ..Comp::default() },
            drive: Drive::default(),
            drive_target: 0.0,
            lofi: Lofi::default(),
            filter: [Biquad::default(); 2],
            filter_mode: s.filter,
            pan_target: 0.0,
            cutoff: 0.0,
            cutoff_set: false,
            eq: Biquad::default(),
            fade: None,
            released: false,
            sustained: false,
        };
        // Prime the interpolator: x[n] .. x[n+4] are the first frames.
        let data = &z.sample.frames;
        for k in 3..8 {
            v.hist[k] = v.stream.next(data);
        }
        v
    }

    /// Note-off for note `id` (§9).
    pub fn note_off(&mut self, id: u64, note: i64) {
        if let Some(a) = self.audio.as_mut() {
            a.note_off(note);
            return;
        }
        if let Some((_, s, ..)) = self.synth.as_mut() {
            s.note_off(note);
            return;
        }
        if self.settings.legato() {
            self.held.retain(|&n| n != note);
            if let Some(&last) = self.held.last() {
                let frames = (self.settings.glide * self.settings.glide * self.sr as f32).round().max(1.0);
                self.glide_target = (last - self.first_note) as f32 / 12.0;
                self.glide_step = (self.glide_target - self.glide) / frames;
                return;
            }
            for v in &mut self.voices {
                v.release(self.pedal);
            }
            return;
        }
        for v in self.voices.iter_mut().filter(|v| v.id == id) {
            v.release(self.pedal);
        }
    }

    /// Release every voice playing `note` (a key let go).
    pub fn release_note(&mut self, note: i64) {
        if let Some(a) = self.audio.as_mut() {
            a.note_off(note);
            return;
        }
        if let Some((_, s, ..)) = self.synth.as_mut() {
            s.note_off(note);
            return;
        }
        let ids: Vec<u64> = self.voices.iter().filter(|v| v.note == note).map(|v| v.id).collect();
        for id in ids {
            self.note_off(id, note);
        }
        self.held.retain(|&n| n != note);
    }

    /// MIDI controllers (§6.3): mod wheel, sustain pedal, all sound off, pitch bend.
    pub fn control(&mut self, kind: MidiKind) {
        if let Some(a) = self.audio.as_mut() {
            a.control(kind);
            return;
        }
        if let Some((_, s, ..)) = self.synth.as_mut() {
            s.control(kind);
            return;
        }
        match kind {
            MidiKind::Controller { number: 1, value } => self.mod_wheel = value as f32 / 127.0,
            MidiKind::Controller { number: 64, value } => {
                let down = value >= 64;
                if self.pedal && !down {
                    for v in self.voices.iter_mut().filter(|v| v.sustained) {
                        v.sustained = false;
                        v.release(false);
                    }
                }
                self.pedal = down;
            }
            MidiKind::Controller { number: 120, .. } => self.fade_all(),
            MidiKind::Controller { .. } => {}
            MidiKind::PitchBend(v) => self.bend = v as f32 / 8192.0 - 1.0,
            MidiKind::ChannelPressure(_) => {}
        }
    }

    /// Fade every voice out over 10 ms (stop, all sound off).
    pub fn fade_all(&mut self) {
        if let Some(a) = self.audio.as_mut() {
            a.stop();
            return;
        }
        if let Some((_, s, ..)) = self.synth.as_mut() {
            s.stop();
            return;
        }
        for v in &mut self.voices {
            v.fade_out(self.sr);
        }
        self.held.clear();
        (self.glide, self.glide_target, self.glide_step) = (0.0, 0.0, 0.0);
        self.pedal = false;
        self.bend = 0.0;
    }

    /// Mix the voices into interleaved `out`; returns the peak (left, right).
    pub fn render(&mut self, out: &mut [f32]) -> (f32, f32) {
        if self.synth.is_some() {
            return self.render_synth(out);
        }
        if let Some(a) = self.audio.as_mut() {
            // Into a cleared buffer: the loop writes its frames (the caller's are mixed).
            let peak = a.render(out);
            self.last_peak = peak.0.max(peak.1);
            return peak;
        }
        let frames = out.len() / 2;
        let mut peak = (0f32, 0f32);
        self.last_peak = 0.0;
        let s = self.settings.clone();
        // Lo-fi settings (§8.4).
        let b = t::lookup(&t::LOFI_BITS_CURVE, s.lofi_bits);
        let levels = (b < 0.99999).then(|| 2f32.powf((16.0 * b).max(1.0)));
        let r = t::lookup(&t::LOFI_RATE_CURVE, s.lofi_rate);
        let hold = (r < 0.99999).then(|| ((self.sr as f32) / (r * self.sr as f32).max(50.0)).max(1.0));
        let base_cutoff = t::lookup(&t::CUTOFF_CURVE, s.cutoff.clamp(0.0, 1.0));
        let q = (1.0 - (1.0 - s.resonance) * (1.0 - s.resonance)).clamp(0.0, 1.0);
        let ramp = ((self.sr * 0.02 / CHUNK as f64).round().max(1.0) * CHUNK as f64) as f32;
        let eq = (s.filter == 4).then(|| peak_eq(s.eq.0, s.eq.1, s.eq.2, self.sr));
        let lfo_scale = (1.0 - s.mw_lfo) + self.mod_wheel * s.mw_lfo;
        let free_inc = self.free_lfo_hz() / self.sr;
        let zones = &self.zones;
        let voices = &mut self.voices;
        let sr = self.sr;
        let coeffs = self.coeffs;
        let comp_target = s.comp.clamp(0.0, 1.0) * 50.0;
        let mw = self.mod_wheel;
        let bend = self.bend * s.bend_range;
        let engine_gain = if s.mode == 1 { 0.8637621 } else { 1.0 };
        let quantize = s.mode >= 1;
        for f in 0..frames {
            // Glide (legato).
            if self.glide_step != 0.0 {
                self.glide += self.glide_step;
                if (self.glide_step > 0.0 && self.glide >= self.glide_target) || (self.glide_step < 0.0 && self.glide <= self.glide_target)
                {
                    self.glide = self.glide_target;
                    self.glide_step = 0.0;
                }
            }
            let glide = self.glide;
            let mut l = 0f32;
            let mut rr = 0f32;
            for v in voices.iter_mut() {
                let z = &zones[v.zone];
                // Another filter type starts from rest, as Maschine's separate filters do.
                if v.filter_mode != s.filter {
                    v.filter_mode = s.filter;
                    v.filter.iter_mut().for_each(Biquad::reset);
                    v.eq.reset();
                    v.cutoff_set = false;
                }
                // Control rate: modulation, pitch, cutoff, drive.
                if v.ctl_left == 0 {
                    v.ctl_left = CHUNK;
                    let lfo = v.lfo(s.lfo_type);
                    let env = v.modenv.level;
                    let octaves = v.octaves + glide + s.lfo_pitch * lfo_scale * lfo + s.env_pitch * env + bend;
                    let ratio = 2f64.powf(octaves as f64) * v.rate_ratio;
                    let ratio = if s.mode == 1 { ratio.min(1.49831) } else { ratio };
                    v.inc = (ratio * 65536.0).round().clamp(1.0, 65536.0 * 64.0) as u32;
                    let dmod = s.env_drive * env + s.lfo_drive * lfo_scale * lfo;
                    let d = (s.drive + dmod).clamp(0.0, 1.0);
                    v.drive_target = if d <= 0.0001 { 0.0 } else { d * 127.0 };
                    if (1..=3).contains(&s.filter) {
                        let cmod = s.env_cutoff * env + s.lfo_cutoff * lfo_scale * lfo + s.vel_cutoff * v.vel + s.mw_cutoff * mw;
                        let target = base_cutoff + cmod;
                        if !v.cutoff_set {
                            v.cutoff = target;
                            v.cutoff_set = true;
                        } else {
                            v.cutoff += (target - v.cutoff) * (CHUNK as f32 / ramp).min(1.0);
                        }
                        let fr = (cutoff_fraction(v.cutoff).min(1.0) * (44100.0 / sr as f32) * 0.998004).clamp(0.0, 1.0);
                        match s.filter {
                            1 => v.filter[0].set(two_pole(fr, q, false)),
                            3 => v.filter[0].set(two_pole(fr, q, true)),
                            _ => {
                                let [(b0, b1, a1), (c0, c1, a2)] = band_pass(fr);
                                v.filter[0].set((b0, b1, 0.0, a1, 0.0));
                                v.filter[1].set((c0, c1, 0.0, a2, 0.0));
                            }
                        }
                    }
                    if let Some(c) = eq {
                        v.eq.set(c);
                    }
                    let pm = s.lfo_pan * lfo_scale * lfo + s.env_pan * env + s.mw_pan * mw;
                    // Hard left to hard right at most (the amounts add up to ±3).
                    v.pan_target = pm.clamp(-1.0, 1.0);
                    v.lfo_phase += v.lfo_inc * CHUNK as f64;
                    if v.lfo_phase >= 1.0 {
                        v.lfo_phase -= v.lfo_phase.floor();
                        v.lfo_seed = v.lfo_seed.wrapping_mul(0x0BB3_8435).wrapping_add(0x3619_636B);
                    }
                }
                v.ctl_left -= 1;
                // Resample (§7).
                let w = weights(1.0 - v.frac as f32 / 65536.0);
                let mut x = [0f32; 2];
                for (k, wk) in w.iter().enumerate() {
                    x[0] += wk * v.hist[k][0];
                    x[1] += wk * v.hist[k][1];
                }
                v.frac += v.inc;
                while v.frac >= 65536 {
                    v.frac -= 65536;
                    v.hist.copy_within(1..8, 0);
                    let mut next = v.stream.next(&z.sample.frames);
                    if quantize {
                        next = [(next[0] * 2048.0).trunc() / 2048.0, (next[1] * 2048.0).trunc() / 2048.0];
                    }
                    v.hist[7] = next;
                    if v.stream.done {
                        v.tail += 1;
                    }
                }
                // Effects (§2, §8).
                x = v.comp.run(x, comp_target, coeffs);
                x = v.drive.run(x, v.drive_target, coeffs.c50);
                x = v.lofi.run(x, levels, hold);
                match s.filter {
                    1 | 3 => x = v.filter[0].run(x),
                    2 => {
                        let y = v.filter[0].run(x);
                        x = v.filter[1].run(y);
                    }
                    4 => x = v.eq.run(x),
                    _ => {}
                }
                // A safety net: a voice whose effects run away starts them again.
                if !(x[0].is_finite() && x[1].is_finite()) || x[0].abs().max(x[1].abs()) > 64.0 {
                    v.filter.iter_mut().for_each(Biquad::reset);
                    v.eq.reset();
                    v.comp = Comp { g: 1.0, ..Comp::default() };
                    v.drive = Drive::default();
                    x = [0.0; 2];
                }
                let amp = if v.amp_on { v.amp.next() } else { 1.0 };
                v.modenv.next();
                // Pan modulation, smoothed (§6.6).
                v.pan_mod = 0.9 * v.pan_mod + 0.1 * v.pan_target;
                let pm = &mut v.pan_mod;
                let mut fade = 1.0;
                if let Some((left, total)) = &mut v.fade {
                    fade = *left as f32 / *total as f32;
                    *left = left.saturating_sub(1);
                }
                let (pl, pr) = if *pm > 0.0 { (1.0 - *pm, 1.0) } else { (1.0, 1.0 + *pm) };
                let k = amp * fade * engine_gain;
                l += x[0] * v.gain[0] * pl * k;
                rr += x[1] * v.gain[1] * pr * k;
            }
            let (a, b) = (l * self.mix.0, rr * self.mix.1);
            out[f * 2] += a;
            out[f * 2 + 1] += b;
            peak = (peak.0.max(a.abs()), peak.1.max(b.abs()));
            self.last_peak = self.last_peak.max(peak.0).max(peak.1);
            // The free-running LFO.
            self.lfo_phase = (self.lfo_phase + free_inc).fract();
            // Voices that ended.
            if f % CHUNK as usize == 0 || f + 1 == frames {
                voices.retain(|v| v.alive());
            }
        }
        peak
    }

    /// [`SamplerRt::render`] for a sound played by a synth.
    fn render_synth(&mut self, out: &mut [f32]) -> (f32, f32) {
        let Some((_, synth, l, r)) = self.synth.as_mut() else { return (0.0, 0.0) };
        let mut peak = (0f32, 0f32);
        let (mut at, len) = (0, out.len());
        while at < len {
            // Gliding to new settings: a step of it every glide::STEP frames.
            let glide = self.synth_glide.as_mut().filter(|g| g.gliding());
            let frames = if glide.is_some() { crate::glide::STEP } else { SYNTH_CHUNK };
            if let Some(g) = glide
                && g.tick()
            {
                synth.set(g.now());
            }
            let chunk = &mut out[at..(at + frames * 2).min(len)];
            at += chunk.len();
            let n = chunk.len() / 2;
            synth.process(&mut l[..n], &mut r[..n]);
            for (f, x) in chunk.chunks_exact_mut(2).enumerate() {
                let (a, b) = (l[f] * self.mix.0, r[f] * self.mix.1);
                x[0] += a;
                x[1] += b;
                peak = (peak.0.max(a.abs()), peak.1.max(b.abs()));
            }
        }
        self.last_peak = peak.0.max(peak.1);
        peak
    }
}

impl Voice {
    fn lfo(&mut self, kind: i64) -> f32 {
        let p = self.lfo_phase;
        self.lfo_value = match kind {
            0 => (std::f64::consts::TAU * p).cos() as f32,
            1 => {
                let q = (p + 0.5).fract();
                if q <= 0.5 { (4.0 * q - 1.0) as f32 } else { (1.0 - 4.0 * (q - 0.5)) as f32 }
            }
            2 => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            3 => (1.0 - 2.0 * p) as f32,
            _ => f32::from_bits((self.lfo_seed & 0x7F_FFFF) | 0x4000_0000) - 3.0,
        };
        self.lfo_value
    }

    fn fade_out(&mut self, sr: f64) {
        if self.fade.is_none() {
            let n = ((sr * 0.01) as u32).max(1);
            self.fade = Some((n, n));
        }
    }

    fn release(&mut self, pedal: bool) {
        if pedal {
            self.sustained = true;
            return;
        }
        if !self.released {
            self.released = true;
            self.amp.release();
            self.modenv.release();
        }
    }

    fn alive(&self) -> bool {
        if matches!(self.fade, Some((0, _))) {
            return false;
        }
        if self.amp_on && self.amp.done() {
            return false;
        }
        // The stream ran out and the interpolator has emptied.
        self.tail < 8
    }
}

#[cfg(test)]
mod tests {
    /// A modulated field takes the value Settings::new would give it.
    #[test]
    fn set_field_matches_new() {
        let sr = 48_000.0;
        let base = deckard_model::Sampler::default();
        for k in super::MODULATED {
            let mut m = base.clone();
            m.set(k, 0.37);
            let mut st = super::Settings::new(&base, sr);
            assert!(st.set_field(k, 0.37, sr), "{k}");
            assert!(st == super::Settings::new(&m, sr), "{k}");
        }
        assert!(!super::Settings::new(&base, sr).set_field("polyphony", 3.0, sr), "not a modulated field");
    }

    use super::*;

    /// The display's response: LP2 passes lows and cuts highs, HP2 the reverse, BP2
    /// peaks near the cutoff, the EQ shows its gain at its frequency.
    #[test]
    fn filter_response_has_the_right_shape() {
        let at = |r: &[(f32, f32)], x: f32| r.iter().min_by(|a, b| (a.0 - x).abs().total_cmp(&(b.0 - x).abs())).unwrap().1;
        let lp = filter_response(1, 0.5, 0.0, (0.5, 0.5, 0.0), 101);
        assert!(at(&lp, 0.1).abs() < 1.0 && at(&lp, 0.9) < -12.0, "LP2 {} {}", at(&lp, 0.1), at(&lp, 0.9));
        let hp = filter_response(3, 0.5, 0.0, (0.5, 0.5, 0.0), 101);
        assert!(at(&hp, 0.9).abs() < 1.5 && at(&hp, 0.1) < -12.0, "HP2 {} {}", at(&hp, 0.1), at(&hp, 0.9));
        let bp = filter_response(2, 0.5, 0.0, (0.5, 0.5, 0.0), 101);
        assert!(at(&bp, 0.5) > at(&bp, 0.05) + 6.0 && at(&bp, 0.5) > at(&bp, 0.95) + 6.0);
        // ±18 dB shown is about ±36 dB at the centre, as Maschine computes it (sampler-dsp.md).
        let eq = filter_response(4, 0.5, 0.0, (0.5, 0.3, 1.0), 101);
        assert!((at(&eq, 0.5) - 36.0).abs() < 1.0 && at(&eq, 0.0).abs() < 2.0, "EQ {}", at(&eq, 0.5));
        let res = filter_response(1, 0.5, 1.0, (0.5, 0.5, 0.0), 201);
        assert!(res.iter().map(|p| p.1).fold(f32::MIN, f32::max) > 6.0, "resonance peaks");
    }

    /// Starting notes (up to the voice limit) never grows the voice list: that would
    /// allocate on the audio thread. The limit still fades the oldest notes.
    #[test]
    fn notes_start_without_allocating() {
        let sample = Arc::new(crate::sample::Sample { rate: 48_000, frames: vec![[0.5, 0.5]; 48_000] });
        let zone = ZoneRt {
            sample,
            root: 60,
            keys: (0, 127),
            vels: (0, 127),
            gain: 1.0,
            pan: 0.0,
            tune: 0.0,
            play: (0, 48_000),
            looped: None,
            x_fade: 0,
            fade_in: 0,
            fade_out: 0,
        };
        let model = deckard_model::Sampler::default();
        let mut smp = SamplerRt::new(Settings::new(&model, 48_000.0), vec![zone], (1.0, 1.0), 48_000.0);
        let ctx = Context { tempo: 120.0, beats: 0.0 };
        let buffer = smp.voices.as_ptr();
        let limit = smp.settings.note_limit();
        for n in 0..VOICE_ROOM.min(limit + 4) as i64 {
            smp.note_on(n as u64 + 1, 40 + n, 100, &ctx);
        }
        assert_eq!(smp.voices.as_ptr(), buffer, "the voice list was not reallocated");
        let sounding = smp.voices.iter().filter(|v| v.fade.is_none()).count();
        assert!(sounding <= limit, "{sounding} notes sound, limit {limit}");
    }

    #[test]
    fn segments_reach_their_end() {
        for c in [-0.332, 0.0, 0.4427] {
            let mut s = Segment::new(0.2, 0.9, 1000, c);
            let mut y = 0.0;
            while s.left > 0 {
                y = s.next();
            }
            // The last value is one step short of the end (the next stage starts there).
            assert!((y - 0.9).abs() < 5e-3, "curvature {c}: {y}");
        }
        // The attack is slow at first, the decay fast.
        let mut a = Segment::new(0.0, 1.0, 1000, C_ATTACK);
        let mid = (0..500).map(|_| a.next()).last().unwrap();
        assert!((0.25..0.4).contains(&mid), "attack at half time: {mid}");
        let mut d = Segment::new(1.0, 0.0, 1000, C_DECAY);
        let mid = (0..500).map(|_| d.next()).last().unwrap();
        assert!((0.1..0.3).contains(&mid), "decay at half time: {mid}");
    }

    #[test]
    fn times_and_displays() {
        let (a, h, d) = envelope_ms(1.0, 1.0, 1.0);
        assert!((7000.0..8500.0).contains(&a) && (7000.0..8500.0).contains(&h), "{a} {h}");
        assert!((11500.0..13500.0).contains(&d), "{d}");
        assert_eq!(envelope_ms(0.0, 0.0, 0.0).0, 0.0);
        let (hz, _) = lfo_rate(0.0);
        assert!((hz - 1.0 / 32.0).abs() < 1e-9);
        assert!((lfo_rate(1.0).0 - 16.0).abs() < 1e-9);
        assert_eq!(lofi_rate_hz(0.0, 44100.0), None, "SR at 0 is off");
        assert!(cutoff_hz(1.0, 1) > 18000.0 && cutoff_hz(0.0, 1) < 200.0);
    }

    #[test]
    fn interpolator_weights_sum_to_one() {
        for i in 0..=16 {
            let w = weights(i as f32 / 16.0);
            let sum: f32 = w.iter().sum();
            assert!((sum - 1.0).abs() < 1e-4, "t = {}: {sum}", i as f32 / 16.0);
        }
        let w = weights(1.0);
        assert!((w[3] - 1.0).abs() < 1e-4 && w[4].abs() < 1e-4, "t = 1 is x[n]: {w:?}");
    }

    #[test]
    fn low_pass_has_unity_gain_at_dc() {
        for q in [0.0, 0.5] {
            let (b0, b1, b2, a1, a2) = two_pole(0.2, q, false);
            let dc = (b0 + b1 + b2) / (1.0 - a1 - a2);
            let want = (1.0 - q as f64) * 0.55 + 0.45;
            assert!((dc - want).abs() < 1e-3, "q {q}: {dc}");
        }
        let (b0, b1, b2, ..) = two_pole(0.2, 0.0, true);
        assert!((b0 + b1 + b2).abs() < 1e-3, "high pass blocks DC");
    }
}
