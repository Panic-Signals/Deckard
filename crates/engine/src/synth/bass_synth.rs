//! Maschine's Bass Synth (docs/synth-bass-synth.md): `DATA::BassSynth` →
//! `RT::DrumSynthWrapper<DSP::BassSynthProcessor>` → `DSP::BassSynthCore`. A monophonic
//! two-oscillator synth: two morphing oscillators (`RxOsc4Wave`, sine → triangle → saw →
//! square), mixed, through a nonlinear 2-pole low-pass (`TSKFilterNLA`) whose cutoff the
//! envelope sweeps, a drive stage (`RxDrive`), and an attack–hold–decay envelope
//! (`RxEnvAHD`) as the amplitude. Below 88 kHz everything runs at twice the sample rate
//! and a half-band converter brings it back down. Notes glide (legato) when Glide is on.
//!
//! Like Maschine, the instrument keeps running once a note has played: its oscillators
//! free-run between notes and later notes start from where they are, so
//! [`Synth::voices`] stays 1 until a choke ([`Synth::stop`]) has faded it out.
//!
//! The kernels (`osc`, `env`, `filter`, `drive`) keep their state laid out as Maschine's
//! compiled code does; the glue below follows `BassSynthCore` operation by operation
//! (`f32`, fused multiply-adds where Maschine fuses, its libm calls), so the output
//! matches bit for bit.

pub(crate) mod drive;
pub(crate) mod env;
pub(crate) mod filter;
pub(crate) mod osc;
pub(crate) mod rx;
mod src;

use super::Synth;
use deckard_model::MidiKind;
use drive::Drive;
use env::Env;
use filter::Filter;
use osc::Osc;
use rx::Exp2Table;
use src::Down;

/// `__exp10` from the system math library, which Maschine calls (10^x).
#[cfg(target_os = "macos")]
fn exp10(x: f64) -> f64 {
    unsafe extern "C" {
        fn __exp10(x: f64) -> f64;
    }
    // SAFETY: a pure libm function.
    unsafe { __exp10(x) }
}

#[cfg(not(target_os = "macos"))]
fn exp10(x: f64) -> f64 {
    10f64.powf(x)
}

/// The module's fields (mxprj names), their ranges and defaults (`BassSynth::BassSynth`),
/// in the processor's parameter order. Integer fields are normalized like the floats.
const PARAMS: [(&str, f32, f32, f32); 13] = [
    ("shape", 0.66, 0.0, 1.0),
    ("octave", -2.0, -3.0, 3.0),
    ("shape_2", 0.66, 0.0, 1.0),
    ("octave_2", -2.0, -3.0, 3.0),
    ("detune", 0.0, 0.0, 1.0),
    ("osc_mix", 0.0, 0.0, 1.0),
    ("cutoff", 0.2, 0.0, 1.0),
    ("resonance", 0.75, 0.0, 1.0),
    ("mod_amount", 0.5, 0.0, 1.0),
    ("decay", 0.5, 0.0, 1.0),
    ("drive", 0.0, -1.0, 1.0),
    ("glide", 0.0, 0.0, 1.0),
    ("glide_time", 0.09090909, 0.0, 1.0),
];

/// The processor's parameter `i` → the core's (core index 6 is unused).
const CORE_INDEX: [usize; 13] = [0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 12, 13];

/// A stored value → the processor's normalized value, as `connectParameters`' functions
/// do: `(v − min)` in `f32`, times `1/(max − min)` in `f64`.
fn normalize(v: f32, min: f32, max: f32) -> f32 {
    ((1.0 / (max as f64 - min as f64)) * (v - min) as f64) as f32
}

/// One of the core's five linear parameter glides (20 ms): cutoff, resonance, mod
/// amount, and the drive's input and output gains.
#[derive(Clone, Copy, Default)]
struct Ramp {
    cur: f32,
    target: f32,
    step: f32,
    /// Snap when within this of the target (0).
    tol: f32,
    /// Ramp length (samples) as a float and its rounding.
    len: f32,
    count: i32,
    reload: i32,
    /// A new target is pending.
    pending: bool,
}

impl Ramp {
    fn aim(&mut self, v: f32) {
        self.target = v;
        if self.cur != v {
            self.step = (v - self.cur) / self.len;
            self.count = self.reload;
            self.pending = true;
        }
    }

    fn snap(&mut self) {
        if self.pending {
            self.cur = self.target;
            self.step = 0.0;
            self.count = 0;
            self.pending = false;
        }
    }

    /// One sample.
    fn tick(&mut self) {
        if (self.target - self.cur).abs() <= self.tol {
            self.snap();
        } else {
            let c = self.count;
            self.count = c.wrapping_sub(1);
            if c < 2 {
                self.snap();
            } else {
                self.cur += self.step;
            }
        }
    }
}

/// The glide of one oscillator's pitch (MIDI note units): linear over Glide Time when
/// Glide is on; with Glide off the pitch jumps, though the interpolator still runs (over
/// 0.1 ms) and later glides start from where it got.
#[derive(Clone, Copy, Default)]
struct PitchGlide {
    /// Oversampled rate.
    sr: f32,
    target: f32,
    /// Glide on (1) or off (0).
    on: f32,
    /// The pitch out.
    out: f32,
    active: bool,
    pos: f32,
    cur: f32,
    start: f32,
    delta: f32,
}

impl PitchGlide {
    /// `onSetSampleRate`: restart at the current target.
    #[allow(clippy::eq_op)]
    fn init(&mut self, sr: f32) {
        let (t, on) = (self.target, self.on);
        *self = PitchGlide { sr, target: t, on, out: t, active: true, pos: 0.0, cur: t, start: t, delta: t - t };
    }

    fn go(&mut self, note: f32) {
        self.target = note;
        self.active = true;
        self.pos = 0.0;
        self.start = self.cur;
        self.delta = note - self.cur;
        self.out = note;
    }

    /// One sample; `t` is Glide Time·0.1 (seconds).
    fn tick(&mut self, t: f32) {
        if self.active {
            let d = t * self.on;
            let d = if d < 1e-4 { 1e-4 } else { d };
            let mut p = self.pos + 1.0 / (d * self.sr);
            if p >= 1.0 {
                self.active = false;
                p = 1.0;
            }
            let v = p * self.delta + self.start;
            self.cur = v;
            self.pos = p;
            if self.on > 0.0 {
                self.out = v;
            }
        }
    }
}

/// `MASCOMMON::SAMPLER::Glide`: the held keys, most recent first.
struct Keys {
    held: [u8; 128],
    n: usize,
}

impl Keys {
    fn press(&mut self, key: u8) {
        if let Some(i) = self.held[..self.n].iter().position(|&k| k == key) {
            self.held.copy_within(0..i, 1);
        } else if self.n < 128 {
            self.held.copy_within(0..self.n, 1);
            self.n += 1;
        } else {
            return;
        }
        self.held[0] = key;
    }

    fn release(&mut self, key: u8) {
        let mut j = 0;
        for i in 0..self.n {
            if self.held[i] != key {
                self.held[j] = self.held[i];
                j += 1;
            }
        }
        self.n = j;
    }

    fn last(&self) -> Option<u8> {
        (self.n > 0).then(|| self.held[0])
    }
}

/// Voice state (`+0x3278`).
#[derive(Clone, Copy, PartialEq)]
enum State {
    Off,
    On,
    /// Choked: fading out.
    Choke,
}

pub struct BassSynth {
    t: Exp2Table,
    sr: f32,
    /// Oversampling factor (2 below 88 kHz, else 1).
    os: u32,
    /// The processor's normalized parameters.
    p: [f32; 13],
    /// Not yet set up: the first [`Synth::set`] applies its values as a project load does.
    fresh: bool,
    osc1: Osc,
    osc2: Osc,
    env: Env,
    drive: Drive,
    filter: Filter,
    /// The half-band down-sampler (both channels carry the same signal).
    down: Down,
    keys: Keys,
    glide1: PitchGlide,
    glide2: PitchGlide,
    /// Cutoff, resonance, mod amount, drive input gain, drive output gain.
    ramps: [Ramp; 5],
    // Core values (`BassSynthCore` +0xb8…).
    shape1: f32,
    shape2: f32,
    detune: f32,
    mix: f32,
    cutoff: f32,
    res: f32,
    drive_amt: f32,
    glide_on: f32,
    glide_time: f32,
    /// The cutoff, mod amount, resonance and envelope the filter was last tuned for.
    last_cut: f32,
    last_res: f32,
    last_mod: f32,
    last_env: f32,
    state: State,
    amp: f32,
    fade: f32,
    /// A note started since the last reset (reset when the voice has ended).
    dirty: bool,
    /// The wrapper's 16-frame segment counter.
    grid: u32,
    /// The next frame starts a segment (a note, choke or parameter event came in).
    boundary: bool,
    /// A note-on was taken since the last `process` (the wrapper takes one per timestamp).
    noted: bool,
    /// The current segment runs the core (its voice was on when it started).
    live: bool,
    /// The current segment runs the glides (one was pending when it started).
    gliding: bool,
    /// The first segment since a reset: it checks the resonance on every sample; later
    /// segments only while its glide is pending.
    first: bool,
}

impl BassSynth {
    pub fn new(sr: f64) -> BassSynth {
        let mut s = BassSynth {
            t: Exp2Table::new(),
            sr: 44100.0,
            os: 2,
            p: [0.0; 13],
            fresh: true,
            osc1: Osc::new(),
            osc2: Osc::new(),
            env: Env::new(),
            drive: Drive::new(),
            filter: Filter::new(),
            down: Down::new(),
            keys: Keys { held: [0; 128], n: 0 },
            glide1: PitchGlide::default(),
            glide2: PitchGlide::default(),
            ramps: [Ramp::default(); 5],
            shape1: 0.0,
            shape2: 0.0,
            detune: 0.0,
            mix: 0.0,
            cutoff: 0.0,
            res: 0.0,
            drive_amt: 0.0,
            glide_on: 0.0,
            glide_time: 0.0,
            last_cut: 10.0,
            last_res: 0.0,
            last_mod: 0.0,
            last_env: 0.0,
            state: State::Off,
            amp: 0.0,
            fade: 0.0,
            dirty: false,
            grid: 0,
            boundary: true,
            noted: false,
            live: false,
            gliding: false,
            first: true,
        };
        s.construct();
        // The wrapper starts at 44.1 kHz; the real rate comes with the first parameters.
        s.set_rate(44100.0);
        s.sr = sr as f32;
        s
    }

    /// `BassSynthCore::BassSynthCore`: the kernels' fixed settings.
    fn construct(&mut self) {
        let t = &self.t;
        self.env.m.set_f(0x1e0, 1.0);
        self.env.m.set_f(0x0, 1.0);
        self.env.end_init(t);
        for o in [&mut self.osc1, &mut self.osc2] {
            o.m.set_f(0x230, 1.0);
            o.m.set_f(0x0, 1.0);
            o.end_init(t);
        }
        self.drive.m.set_f(0x8c, 1.0);
        self.drive.m.set_f(0x0, 1.0);
        self.drive.end_init(t);
        self.glide1.sr = 1.0;
        self.glide2.sr = 1.0;
        // The filter's period, coefficient and frequency start as placeholders.
        self.filter.m.set_f(0x0, 1.0);
        self.filter.m.set_f(0x4, -1.0);
        self.filter.m.set_f(0x8, -1.0);
        self.reset();
        let t = &self.t;
        // Attack 13 (4.5 ms), hold 30 (32 ms), attack curve 3, decay curve −10, velocity
        // scaling 1 (§4.3).
        self.env.m.set_f(0x18, 13.0);
        self.env.ev_a(t);
        self.env.m.set_f(0x30, 30.0);
        self.env.ev_h(t);
        self.env.m.set_f(0x20, 3.0);
        self.env.ev_ax();
        self.env.m.set_f(0x40, -10.0);
        self.env.ev_dx();
        // Two of the envelope's mode inputs, 0 (their events inlined in the original).
        self.env.m.set_f(0x60, 0.0);
        self.env.m.set_d(0x140, 0.0);
        self.env.m.set_d(0x1a0, 2.0);
        self.env.m.set_f(0x58, 0.0);
        self.env.m.set_f(0x10, 1.0);
        self.env.ev_vel_s();
        self.drive.m.set_f(0x1c, 1.0);
        self.drive.m.set_f(0x70, 1.0);
        self.drive.m.set_f(0x14, 20.0);
        self.drive.m.set_f(0x68, 20.0);
        // The filter's nonlinearity scales (+0x10…+0x20).
        self.filter.m.set_f(0x10, 4.0);
        self.filter.m.set_f(0x14, 0.25);
        self.filter.m.set_f(0x18, 3.4);
        self.filter.m.set_f(0x1c, f32::from_bits(0x3e96_9696));
        self.filter.m.set_f(0x20, f32::from_bits(0x3f4c_cccd));
    }
}

impl BassSynth {
    /// `BassSynthCore::onSetSampleRate`: the oversampling, every kernel restarted at the
    /// new rate (keeping its inputs), the 20 ms parameter glides and the choke fade.
    /// Rates below 44.1 kHz are refused, as in Maschine.
    fn set_rate(&mut self, sr: f32) {
        if sr < 44100.0 {
            return;
        }
        self.os = if sr < 88000.0 { 2 } else { 1 };
        let r = self.os as f32 * sr;
        self.down.clear();
        let t = &self.t;
        for o in [&mut self.osc1, &mut self.osc2] {
            o.m.set_f(0x230, r);
            restart_osc(o, r, t);
        }
        self.env.m.set_f(0x1e0, r);
        restart_env(&mut self.env, r, t);
        self.drive.m.set_f(0x8c, r);
        restart_drive(&mut self.drive, r, t);
        self.glide1.init(r);
        self.glide2.init(r);
        let period = 1.0 / r;
        if period != self.filter.m.f(0x0) {
            self.filter.m.set_f(0x0, period);
            let f = self.filter.m.f(0x8);
            self.filter.set_freq(f);
        }
        self.state = State::Off;
        self.amp = 0.0;
        // Choke fade: halves every 50 ms.
        let x = (1.0 / r) * f32::from_bits(0xbf31_7215) / 0.05;
        let x = x.exp();
        self.fade = if x > 1.0 { 1.0 } else { x };
        let len = r * f32::from_bits(0x3a83_126f) * 20.0;
        let n = len.round() as i32;
        for g in &mut self.ramps {
            g.len = len;
            g.reload = n;
            g.count = n;
            g.tol = 0.0;
        }
        self.reset();
    }

    /// `BassSynthCore::onReset`: the oscillators, envelope and drive restarted from their
    /// inputs, filter and converter cleared, pending glides finished, the drive's output
    /// gain back at 1 (until the drive is next set).
    fn reset(&mut self) {
        self.dirty = false;
        self.first = true;
        self.last_env = 0.0;
        self.down.clear();
        (self.last_cut, self.last_res) = (-1.0, -1.0);
        let t = &self.t;
        for o in [&mut self.osc1, &mut self.osc2] {
            let r = o.m.f(0x230);
            restart_osc(o, r, t);
        }
        let r = self.env.m.f(0x1e0);
        restart_env(&mut self.env, r, t);
        let r = self.drive.m.f(0x8c);
        restart_drive(&mut self.drive, r, t);
        self.filter.m.zero(0x30, 0x14);
        self.state = State::Off;
        self.amp = 0.0;
        for g in &mut self.ramps {
            g.snap();
        }
        let g = &mut self.ramps[4];
        (g.cur, g.target, g.step, g.count) = (1.0, 1.0, 0.0, 0);
    }

    /// `BassSynthCore::setParameter` (core index `i`, normalized `v`).
    fn core_set(&mut self, i: usize, v: f32) {
        let t = &self.t;
        match i {
            0 | 2 => {
                let o = if i == 0 { &mut self.osc1 } else { &mut self.osc2 };
                if i == 0 {
                    self.shape1 = v;
                } else {
                    self.shape2 = v;
                }
                // The oscillator glides its shape over 100 ms.
                o.m.set_f(0x8, v);
                o.m.set_i(0x24, 1);
                o.m.set_f(0x28, 0.0);
                let cur = o.m.f(0x2c);
                o.m.set_f(0x30, cur);
                o.m.set_f(0x34, v - cur);
            }
            1 | 3 => {
                // −3..3 octaves, rounded half away from zero.
                let x = v.mul_add(6.0, -3.0).round();
                let o = if i == 1 { &mut self.osc1 } else { &mut self.osc2 };
                o.m.set_f(0x14, x);
                o.m.set_f(0x4c, x);
            }
            4 => self.detune = v,
            5 => self.mix = v,
            7 => {
                // Cutoff as a MIDI pitch, 10..138.
                let c = if v > 1.0 { 1.0 } else { v };
                let c = if v < 0.0 { 0.0 } else { c };
                let x = (c as f64).mul_add(128.0, 10.0) as f32;
                self.cutoff = x;
                self.ramps[0].aim(x);
                self.last_cut = -1.0;
                self.ramps[1].aim(self.res);
                self.last_res = -1.0;
            }
            8 => {
                self.res = v * f32::from_bits(0x3f99_999a); // 1.2
                self.ramps[1].aim(self.res);
                self.last_res = -1.0;
            }
            9 => self.ramps[2].aim(v),
            10 => {
                self.env.m.set_f(0x38, v.mul_add(40.0, 50.0));
                self.env.ev_d(t);
            }
            11 => self.set_drive(v),
            12 => {
                self.glide_on = v;
                self.glide1.on = v;
                self.glide2.on = v;
                self.env.m.set_f(0x50, v);
                self.env.ev_glide();
            }
            13 => {
                let c = if v > 1.0 { 1.0 } else { v };
                let c = if v < 0.0 { 0.0 } else { c };
                self.glide_time = (c as f64).mul_add(f32::from_bits(0x411e_6666) as f64, 0.1f32 as f64) as f32;
            }
            _ => {}
        }
    }

    /// `BassSynthCore::setParamDrv`: Drive −1..1. Positive drives the saturator (its
    /// amount glides over 100 ms inside `RxDrive`); negative raises the input gain up to
    /// +20 dB; the output gain falls by 7 dB (positive) or 10 dB (negative) per unit.
    fn set_drive(&mut self, v: f32) {
        let x = v.mul_add(2.0, -1.0);
        self.drive_amt = x;
        let pos = x.max(0.0);
        let d = &mut self.drive.m;
        d.set_f(0xc, pos);
        d.set_i(0x54, 1);
        d.set_f(0x58, 0.0);
        let cur = d.f(0x5c);
        d.set_f(0x60, cur);
        d.set_f(0x64, pos - cur);
        let db = if x < 0.0 { x * -20.0 } else { -0.0 };
        let g = exp10(db as f64 * 0.05) as f32;
        self.ramps[3].aim(g);
        let a = x.abs();
        let k: f32 = if x >= 0.0 { -7.0 } else { -10.0 };
        let a = if a > 1.0 { 1.0 } else { a };
        let db = k.mul_add(a, 0.0);
        let g = exp10(db as f64 * 0.05) as f32;
        self.ramps[4].aim(g);
    }

    /// `BassSynthProcessor::setParameter` (processor index `i`).
    fn proc_set(&mut self, i: usize, v: f32) {
        self.p[i] = v;
        self.core_set(CORE_INDEX[i], v);
    }
}

/// `onSetSampleRate`/`onReset` for an oscillator: cleared but for its rate, pitch,
/// octave and shape, which it then takes as fresh events.
fn restart_osc(o: &mut Osc, r: f32, t: &Exp2Table) {
    let (shape, pitch, oct) = (o.m.f(0x8), o.m.f(0x10), o.m.f(0x14));
    o.m.zero(0x4, 0x22c);
    o.m.set_f(0x0, r);
    o.m.set_f(0x10, pitch);
    o.m.set_f(0x14, oct);
    o.m.set_f(0x18, 1.0);
    o.m.set_f(0x8, shape);
    o.m.set_f(0xc, 1.0);
    o.end_init(t);
}

/// The same for the envelope: its settings survive (and are taken as fresh events).
fn restart_env(e: &mut Env, r: f32, t: &Exp2Table) {
    const KEEP: [usize; 9] = [0x18, 0x30, 0x38, 0x20, 0x40, 0x60, 0x58, 0x10, 0x50];
    let v = KEEP.map(|o| e.m.f(o));
    e.m.zero(0, 0x1e0);
    e.m.set_f(0x0, r);
    for (o, x) in KEEP.iter().zip(v) {
        e.m.set_f(*o, x);
        e.m.set_f(*o + 4, 1.0);
    }
    e.end_init(t);
}

/// The same for the drive.
fn restart_drive(d: &mut Drive, r: f32, t: &Exp2Table) {
    const KEEP: [usize; 3] = [0xc, 0x1c, 0x14];
    let v = KEEP.map(|o| d.m.f(o));
    d.m.zero(0x4, 0x88);
    d.m.set_f(0x0, r);
    for (o, x) in KEEP.iter().zip(v) {
        d.m.set_f(*o, x);
        d.m.set_f(*o + 4, 1.0);
    }
    d.end_init(t);
}

impl BassSynth {
    /// `BassSynthProcessor::setNoteOn`: all parameters are sent to the core again, then
    /// the key is pressed: a free voice takes its parameters' targets at once; the
    /// pitch glides (or jumps) to the key; the envelope restarts (or, with Glide, carries
    /// on legato).
    fn press(&mut self, key: u32, vel: f32) {
        for (i, &c) in CORE_INDEX.iter().enumerate() {
            self.core_set(c, self.p[i]);
        }
        if !(0.0..=1.0).contains(&vel) {
            return;
        }
        self.keys.press(key as u8);
        if self.state == State::Off {
            for g in &mut self.ramps {
                g.snap();
            }
        }
        self.dirty = true;
        let n = key as f32;
        self.glide1.go(n);
        self.glide2.go(n - self.detune);
        self.env.m.set_f(0x28, 1.0);
        self.env.ev_note_on();
        self.env.m.set_f(0x8, vel);
        self.env.ev_note_vel();
        self.state = State::On;
        self.amp = 0.0;
    }

    /// `BassSynthCore::setNoteOff`. Without Glide nothing happens: the envelope is a
    /// one-shot. With Glide the pitch returns to the most recent key still held, and
    /// letting go of the last one releases the envelope.
    fn release(&mut self, key: u32) {
        self.keys.release(key as u8);
        if self.glide_on == 0.0 {
            return;
        }
        if let Some(k) = self.keys.last() {
            let n = k as f32;
            self.glide1.go(n);
            self.glide2.go(n - self.detune);
        } else {
            self.env.m.set_f(0x48, 0.0);
            self.env.ev_note_off();
        }
    }

    /// `BassSynthCore::onChoke` (MIDI All Sound Off): a sounding voice fades out, halving
    /// every 50 ms, and ends below −80 dB.
    fn choke(&mut self) {
        self.last_env = 0.0;
        if self.state == State::On {
            self.state = State::Choke;
            self.amp = 1.0;
        }
    }

    /// The filter's cutoff (as a frequency) and coefficients for a cutoff pitch, envelope
    /// and modulation amount: pitch = cutoff + 128·env·mod, at most 138.
    fn retune(&mut self, cut: f32, modv: f32, e: f32) {
        let x = (e * modv).mul_add(128.0, cut);
        let x = x.min(138.0);
        let d = (x as f64).mul_add(f64::from_bits(0x3fb5_5555_5555_5555), f64::from_bits(0x4008_4039_7fa8_81bf));
        let f = d.exp2() as f32;
        self.filter.set_freq(f);
        self.last_cut = cut;
        self.last_mod = modv;
    }

    /// One sample at the (over)sampled rate.
    fn tick(&mut self) -> f32 {
        if self.gliding {
            for g in &mut self.ramps {
                g.tick();
            }
        }
        self.env.process(&self.t);
        let e = self.env.out();
        let gt = self.glide_time * f32::from_bits(0x3dcc_cccd);
        self.glide1.tick(gt);
        self.glide2.tick(gt);
        self.osc1.m.set_f(0x10, self.glide1.out);
        self.osc1.process(&self.t);
        let mut a = self.osc1.m.f(0x1c) * f32::from_bits(0x3ecc_cccd);
        self.osc2.m.set_f(0x10, self.glide2.out);
        self.osc2.process(&self.t);
        if self.mix != 0.0 {
            // Equal-power crossfade.
            let b = self.osc2.m.f(0x1c) * f32::from_bits(0x3ecc_cccd);
            let k = (1.0 - self.mix).sqrt();
            let s = self.mix.sqrt() * b;
            a = k.mul_add(a, s);
        }
        let gain = self.ramps[3].cur;
        let cut = self.ramps[0].cur;
        let modv = self.ramps[2].cur;
        if cut != self.last_cut || modv != self.last_mod || (modv != 0.0 && e != self.last_env) {
            self.retune(cut, modv, e);
        }
        self.last_env = e;
        let r = self.ramps[1].cur;
        if (self.first || (self.gliding && self.ramps[1].pending)) && r != self.last_res {
            self.filter.m.set_f(0xc, if r < 2.0 { r + r } else { 4.0 });
            let f = self.filter.m.f(0x8);
            self.filter.set_freq(f);
            self.last_res = r;
        }
        self.filter.process_input(a * gain);
        let y = self.filter.out();
        self.drive.m.set_f(0x8, y * f32::from_bits(0x3f33_3333));
        self.drive.process(&self.t);
        let d = self.drive.m.f(0x24);
        let amp = match self.state {
            State::Off => 0.0,
            State::On => 1.0,
            State::Choke => {
                self.amp *= self.fade;
                if (self.amp as f64) < 1e-4 {
                    self.amp = 0.0;
                    self.state = State::Off;
                }
                self.amp
            }
        };
        e * d * amp * self.ramps[4].cur
    }
}

impl Synth for BassSynth {
    fn set(&mut self, params: &[(String, f64)]) {
        for (i, &(name, d, min, max)) in PARAMS.iter().enumerate() {
            let mut v = params.iter().find(|(n, _)| n == name).map_or(d, |(_, v)| *v as f32);
            if name == "glide" && !params.iter().any(|(n, _)| n == name) {
                // Projects of version 0 store Glide as a switch.
                if let Some((_, l)) = params.iter().find(|(n, _)| n == "legacy_glide") {
                    v = *l as f32;
                }
            }
            let v = if min == -3.0 { v.round() } else { v };
            let n = normalize(v, min, max);
            if self.fresh || n.to_bits() != self.p[i].to_bits() {
                self.proc_set(i, n);
                self.boundary = true;
            }
        }
        if self.fresh {
            // As a project loads: the parameters, then the real rate, then a reset.
            self.fresh = false;
            self.set_rate(self.sr);
            self.reset();
        }
    }

    fn note_on(&mut self, note: i64, velocity: i64) {
        if velocity <= 0 {
            return self.note_off(note);
        }
        // The wrapper takes one note-on per timestamp.
        if self.noted {
            return;
        }
        self.noted = true;
        self.boundary = true;
        self.press((note & 0x7f) as u32, velocity.min(127) as f32 * 0.007874016);
    }

    fn note_off(&mut self, note: i64) {
        self.boundary = true;
        self.release((note & 0x7f) as u32);
    }

    fn stop(&mut self) {
        self.boundary = true;
        self.choke();
    }

    /// DrumSynthWrapper reads only controller 120 (All Sound Off), as a choke; pitch
    /// bend, the mod wheel and the sustain pedal do not reach the Bass Synth.
    fn control(&mut self, kind: MidiKind) {
        if let MidiKind::Controller { number: 120, .. } = kind {
            self.stop();
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.noted = false;
        for (ol, or) in l.iter_mut().zip(r.iter_mut()) {
            // DrumSynthWrapper runs the core in segments of at most 16 frames (and
            // splits them at events); an ended voice is reset at a segment's start.
            if self.grid == 0 {
                self.grid = 16;
                self.boundary = true;
            }
            self.grid -= 1;
            if self.boundary {
                self.boundary = false;
                if self.live {
                    // `processAudioImpl<…, true>` hands over to `<…, false>`.
                    self.first = false;
                }
                if self.state == State::Off && self.dirty {
                    self.reset();
                }
                self.live = self.state != State::Off;
                self.gliding = self.ramps.iter().any(|g| g.pending);
            }
            if !self.live {
                (*ol, *or) = (0.0, 0.0);
                continue;
            }
            let mut y = 0.0;
            for _ in 0..self.os {
                let x = self.tick();
                if self.os == 1 {
                    y = x;
                } else if let Some(v) = self.down.push(x) {
                    y = v;
                }
            }
            (*ol, *or) = (y, y);
        }
    }

    fn voices(&self) -> usize {
        (self.state != State::Off) as usize
    }
}

/// What a new Bass Synth holds: Maschine's defaults (stored values, by field).
pub fn defaults() -> Vec<(&'static str, f64)> {
    vec![
        ("shape", 0.66),
        ("octave", -2.0),
        ("shape_2", 0.66),
        ("octave_2", -2.0),
        ("detune", 0.0),
        ("osc_mix", 0.0),
        ("cutoff", 0.2),
        ("resonance", 0.75),
        ("mod_amount", 0.5),
        ("decay", 0.5),
        ("drive", 0.0),
        ("glide_time", 0.090_909_1),
        ("glide", 0.0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(f: &[(&str, f64)]) -> Vec<(String, f64)> {
        let mut v: Vec<(String, f64)> = PARAMS.iter().map(|p| (p.0.to_string(), p.1 as f64)).collect();
        for (n, x) in f {
            v.iter_mut().find(|p| p.0 == *n).unwrap().1 = *x;
        }
        v
    }

    fn render(s: &mut BassSynth, frames: usize, block: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames];
        let mut r = vec![0.0; block];
        for c in out.chunks_mut(block) {
            let n = c.len();
            s.process(c, &mut r[..n]);
            assert_eq!(c, &r[..n], "both channels carry the same signal");
        }
        out
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn defaults_sound_and_decay() {
        for sr in [44100.0, 48000.0, 96000.0] {
            let mut s = BassSynth::new(sr);
            s.set(&params(&[]));
            assert_eq!(s.voices(), 0);
            assert!(render(&mut s, 1000, 256).iter().all(|&v| v == 0.0), "silent before a note");
            s.note_on(48, 100);
            let a = render(&mut s, sr as usize / 2, 512);
            assert!(rms(&a) > 0.01, "sounds at {sr}");
            assert!(a.iter().all(|v| v.is_finite() && v.abs() < 4.0));
            // A one-shot: it decays while the key is held, and the voice keeps running.
            let b = render(&mut s, 20 * sr as usize, 512);
            assert!(rms(&b[b.len() - 4800..]) < 1e-5);
            assert_eq!(s.voices(), 1);
        }
    }

    #[test]
    fn choke_ends_the_voice_in_silence() {
        let mut s = BassSynth::new(48000.0);
        s.set(&params(&[("drive", 0.5)]));
        s.note_on(40, 127);
        render(&mut s, 4800, 64);
        s.stop();
        render(&mut s, 48000, 100);
        assert_eq!(s.voices(), 0);
        assert!(render(&mut s, 4800, 100).iter().all(|&v| v == 0.0));
        // And it plays again.
        s.note_on(40, 127);
        assert!(rms(&render(&mut s, 4800, 100)) > 0.01);
    }

    #[test]
    fn every_setting_renders_finite() {
        for (n, lo, hi) in PARAMS.iter().map(|p| (p.0, p.2, p.3)) {
            for v in [lo, (lo + hi) * 0.5, hi] {
                let mut s = BassSynth::new(48000.0);
                s.set(&params(&[(n, v as f64), ("osc_mix", 0.5)]));
                s.note_on(36, 100);
                let a = render(&mut s, 9600, 333);
                s.note_on(72, 64);
                s.note_off(36);
                let b = render(&mut s, 9600, 333);
                assert!(a.iter().chain(&b).all(|x| x.is_finite() && x.abs() < 8.0), "{n}={v}");
                assert!(rms(&a) > 1e-4, "{n}={v} sounds");
            }
        }
    }

    #[test]
    fn edits_mid_note_do_not_click() {
        let mut s = BassSynth::new(48000.0);
        s.set(&params(&[("decay", 1.0)]));
        s.note_on(40, 100);
        let a = render(&mut s, 9600, 128);
        s.set(&params(&[("decay", 1.0), ("cutoff", 0.6), ("drive", 0.8), ("shape", 0.1)]));
        let b = render(&mut s, 9600, 128);
        // Cutoff and drive glide over 20 ms, the shape over 100 ms: no step at the edit.
        let jump = (b[0] - a[a.len() - 1]).abs();
        let typical = a[a.len() - 64..].windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(jump <= 2.0 * typical + 1e-3, "{jump} vs {typical}");
        assert_eq!(s.voices(), 1);
    }

    #[test]
    fn glide_is_legato() {
        let mut s = BassSynth::new(48000.0);
        s.set(&params(&[("glide", 1.0), ("glide_time", 0.5)]));
        s.note_on(36, 100);
        render(&mut s, 4800, 256);
        s.note_on(48, 100);
        render(&mut s, 4800, 256);
        // Releasing the newer key returns to the held one; releasing the last one lets
        // the envelope go.
        s.note_off(48);
        assert_eq!(s.keys.last(), Some(36));
        s.note_off(36);
        let a = render(&mut s, 48000 * 12, 512);
        assert!(rms(&a[a.len() - 4800..]) < 1e-5);
    }

    #[test]
    fn block_size_does_not_matter() {
        let run = |block: usize| {
            let mut s = BassSynth::new(44100.0);
            s.set(&params(&[("osc_mix", 0.3), ("detune", 0.2)]));
            s.note_on(45, 90);
            render(&mut s, 20000, block)
        };
        let a = run(1);
        assert_eq!(a, run(17));
        assert_eq!(a, run(4096));
    }

    #[test]
    fn keys_most_recent_first() {
        let mut k = Keys { held: [0; 128], n: 0 };
        k.press(1);
        k.press(2);
        k.press(1);
        assert_eq!(&k.held[..k.n], &[1, 2]);
        k.release(1);
        assert_eq!(k.last(), Some(2));
        k.release(2);
        assert_eq!(k.last(), None);
    }
}
