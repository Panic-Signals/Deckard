//! `NI::DSP::ClassicSnareCore` (docs/synth-snare.md): the Volt (core mode 0 "Tonal", 1
//! "Punchy") and Bit (core mode 2) snares.
//!
//! Signal flow, per sample:
//! - Two sines with a common pitch: `f = note2hz(base + depth·penv)` where `penv` is a
//!   one-pole decay to 0 (time from tune/punch). Each oscillator is a triangle phase fed
//!   through a 7th-order sine polynomial (osc 1 at `1.75129·f`, osc 2 at `f`). Osc 2 is
//!   crossfaded towards `1.4·velocity` by a 1 ms click envelope.
//!   `osc = amp·(amp²·mix·sin1 + (1−mix)·sin2')`, `amp` the amp FreeEnvelope × velocity
//!   peak. A de-click crossfade continues the previous note's slope at a retrigger.
//! - Noise: an LCG through a 9 kHz one-pole low-pass, two TPT state-variable filters in
//!   series (k = 1.6) at `note2hz(base + 53 + colour)`; `bp2 + colfac·srfac·0.4·hp2`,
//!   ×20, × the noise envelope (FreeEnvelope × velocity, max'd with a decaying peak hold,
//!   faded out over 15 ms below 1e-4), hard-clipped to ±4 and softly saturated.
//! - Volt: `osc + amount·(0.65+0.35·vel)·noise`. Bit: the noise+osc sum goes through a
//!   dead zone, a comb (delay `1/note2hz(colour)`), a sample-and-hold "bit crusher" pair
//!   at grit-dependent rates and a third SVF (k = 1.4); `0.75·amount·bp3 + S&H(osc)`.
//! - Optional gate envelope (GATE switch, released by note-off), a one-pole high-pass at
//!   `note2hz(base − 5)`, × 0.55015 (Volt) / 0.47367 (Bit) × the choke envelope.
//!
//! All 16 targets of the parameter smoother ramp over `round(0.02·sr/128)` control steps.
//! Maschine compiles 16 variants of the sample loop (`processAudioImpl<bit, gate, ramp,
//! dirty>`); the non-dirty ones skip coefficient updates for values that do not ramp,
//! which this port reproduces (see [`Upd`]). Every operation follows the arm64 assembly
//! (`mul_add` exactly where it fuses).

mod env;

use super::Core;
use crate::fx::exp_e;
use env::{END, FreeEnv, LN8, TINY};

/// `π` as Maschine writes it (`0x40490fdb`).
const PI: f32 = f32::from_bits(0x4049_0fdb);
/// `x ↦ E(9.965784·x + 602.0529)`: dB → linear × 0.001 (decay times in s).
const DB_K: f32 = f32::from_bits(0x411f_73da);
const DB_C: f32 = f32::from_bits(0x4416_8363);
/// `x ↦ E(5·x + 1381.8816)`: MIDI note → Hz.
const HZ_C: f32 = f32::from_bits(0x44ac_bc36);
/// `−ln 2` and the decay-time → per-sample factor mapping `E(86.5617·x + 1200)`.
const LN2_NEG: f32 = f32::from_bits(0xbf31_7215);
const DECAY_K: f32 = f32::from_bits(0x42ad_1f97);
/// 1e-30, keeps the factor division finite.
const TINY_T: f32 = f32::from_bits(0x0da2_4260);
/// The core's envelope threshold 1e-4 (f32).
const THR: f32 = f32::from_bits(0x38d1_b717);

#[inline]
fn hz(note: f32) -> f32 {
    exp_e(note.mul_add(5.0, HZ_C))
}

#[inline]
fn db_time(x: f32) -> f32 {
    exp_e(x.mul_add(DB_K, DB_C))
}

#[inline]
fn clamp01(x: f32) -> f32 {
    let x = if x < 0.0 { 0.0 } else { x };
    if x > 1.0 { 1.0 } else { x }
}

#[inline]
fn clamp_pm1(x: f32) -> f32 {
    let x = if x < -1.0 { -1.0 } else { x };
    if x > 1.0 { 1.0 } else { x }
}

/// `fcsel gt`: values above 1 become 1.
#[inline]
fn min1(x: f32) -> f32 {
    if x > 1.0 { 1.0 } else { x }
}

/// `x − trunc(x + 0.5)` as the oscillators wrap their phase.
#[inline]
fn wrap(x: f32) -> f32 {
    x - ((x + 0.5) as i32) as f32
}

/// Maschine's 11th-order sine of a phase in turns (wrapped to [−0.5, 0.5] in `f64`).
#[inline]
fn sine(p: f32) -> f32 {
    let half = if p < 0.0 { -0.5f64 } else { 0.5 };
    let w = p - ((half + p as f64) as i32) as f32;
    let y = (w + w) * PI;
    let y2 = y * y;
    let s = y2.mul_add(f32::from_bits(0xb2cd_4ca8), f32::from_bits(0x3638_b94f));
    let s = s.mul_add(y2, f32::from_bits(0xb950_0c03));
    let s = s.mul_add(y2, f32::from_bits(0x3c08_8885));
    let s = s.mul_add(y2, f32::from_bits(0xbe2a_aac1));
    let s = s.mul_add(y2, 1.0);
    y * s
}

/// The prewarped `tan(g)` the filters use: `g + g³/3 + 2g⁵/15`.
#[inline]
fn tan_g(g: f32) -> f32 {
    let g2 = g * g;
    let g3 = g * g2;
    let g5 = g2 * g3;
    g + g5.mul_add(f32::from_bits(0x3e08_8886), g3 * f32::from_bits(0x3eaa_aaaa))
}

/// `exp2` approximation (peak-hold factor): `x` rounded to nearest even, a cubic on the
/// fraction, the integer part into the exponent bits.
#[inline]
fn fast_exp2(x: f32) -> f32 {
    let r = x.round_ties_even();
    let i = if r < 2_147_483_648.0 && r > -2_147_483_648.0 { r as i32 } else { i32::MIN };
    let f = x - i as f32;
    let scale = f32::from_bits((i << 23).wrapping_add(0x3f80_0000) as u32);
    let p = f.mul_add(f32::from_bits(0x3d65_6042), f32::from_bits(0x3e77_ced9));
    let p = p.mul_add(f, f32::from_bits(0x3f31_6873));
    let p = p.mul_add(f, 1.0);
    p * scale
}

/// A TPT state-variable filter (`+0x168`, `+0x198`, `+0x1c8`): `(hp, bp)` per sample.
#[derive(Clone, Copy)]
struct Svf {
    ic1: f32,
    ic2: f32,
    g: f32,
    k: f32,
    cutoff: f32,
    d: f32,
    sr: f32,
    inv: f32,
}

impl Svf {
    fn new(k: f32, sr: f32, inv: f32) -> Svf {
        Svf { ic1: 0.0, ic2: 0.0, g: 0.0, k, cutoff: 1.0, d: 1.0, sr, inv }
    }

    /// The cutoff setter: clamped to [0, 0.49·sr].
    fn set(&mut self, fc: f32) {
        let lim = self.sr * f32::from_bits(0x3efa_e148);
        let c = if fc < 0.0 { 0.0 } else { fc };
        let c = if c > lim { lim } else { c };
        self.cutoff = c;
        let g = tan_g(self.inv * (c * PI));
        self.g = g;
        self.d = 1.0 / g.mul_add(self.k + g, 1.0);
    }

    /// Recompute when `fc` differs from the stored (clamped) cutoff.
    #[inline]
    fn update(&mut self, fc: f32) {
        if self.cutoff != fc {
            self.set(fc);
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> (f32, f32) {
        let (g, k) = (self.g, self.k);
        let i2 = self.ic1 + self.ic1;
        let a = self.ic2.mul_add(2.0, g * i2);
        let b = g.mul_add(k, g * g);
        let c = i2.mul_add(k, a);
        let hp = x - self.d * x.mul_add(b, c);
        let v1 = g.mul_add(hp, self.ic1);
        let bp = self.ic1 + v1;
        let v2 = g.mul_add(bp, self.ic2);
        self.ic1 = v1;
        self.ic2 = v2;
        (hp, bp)
    }
}

/// A one-pole TPT section: the noise low-pass (+0x148) and the output high-pass (+0x1f8).
#[derive(Clone, Copy)]
struct OnePole {
    s: f32,
    a: f32,
    b: f32,
    g: f32,
    cutoff: f32,
    inv: f32,
}

impl OnePole {
    fn set(&mut self, fc: f32) {
        self.cutoff = fc;
        let g = tan_g((fc * PI) * self.inv);
        self.g = g;
        self.a = 1.0 / (g + 1.0);
        self.b = g + -1.0;
    }

    /// `y = a·(x − b·s)`, `s = y`; returns `y`.
    #[inline]
    fn step(&mut self, x: f32) -> f32 {
        let y = self.a * (-self.b).mul_add(self.s, x);
        self.s = y;
        y
    }
}

/// Sample-and-hold (`+0x80240`, `+0x8025c`): holds its input whenever the phase wraps.
#[derive(Clone, Copy)]
struct Hold {
    phase: f32,
    inc: f32,
    held: f32,
    inv: f32,
}

impl Hold {
    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let mut p = self.inc + self.phase;
        let h = if p > 0.5 {
            self.held = x;
            p += -1.0;
            x
        } else {
            self.held
        };
        let p = if p < -0.5 { -0.5 } else { p };
        self.phase = if p > 0.5 { 0.5 } else { p };
        // The mix (+0x18) is 1.
        (h - x).mul_add(1.0, x)
    }
}

/// A 65536-sample delay line (`UTIL::Delay1p`).
struct Delay {
    buf: Vec<f32>,
    w: u32,
    r: u32,
    sr: f32,
}

impl Delay {
    /// `setDelayTime(s)`: rounded half away from zero; out-of-range times are ignored.
    fn set(&mut self, time: f32) {
        let d = time * self.sr;
        let h = if d < 0.0 { -0.5f64 } else { 0.5 };
        let n = (h + d as f64) as i32 as u32;
        if n < 0x10000 {
            self.r = self.w.wrapping_sub(n) & 0xffff;
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        self.buf[self.w as usize] = x;
        let y = self.buf[self.r as usize];
        self.w = (self.w + 1) & 0xffff;
        self.r = (self.r + 1) & 0xffff;
        y
    }
}

/// Which coefficient updates a block runs: `Full` (`processAudioImpl<…, dirty = true>`,
/// after note-on / reset / gate switch), `Ramp` (`<…, ramp = true, dirty = false>`: only
/// for values whose smoother step is non-zero), `Skip` (`<…, false, false>`: none).
#[derive(Clone, Copy, PartialEq)]
enum Upd {
    Full,
    Ramp,
    Skip,
}

// Smoother lanes (+0x80550 current, +0x80590 step, +0x805d0 target).
const OSC2: usize = 0; // sine at f: level 1 − mix (Volt Tonal) / 1
const OSC1: usize = 1; // sine at 1.75·f: level mix (Volt Tonal) / 0
const PTIME: usize = 2; // pitch envelope time (s)
const PDEPTH: usize = 3; // pitch envelope depth (semitones)
const SH_NOTE: usize = 4; // Bit: S&H rate (note)
const F3_NOTE: usize = 5; // Bit: third filter (note)
const DTIME: usize = 6; // Bit: comb delay (s)
const BASE: usize = 7; // base note
const HP_HZ: usize = 8; // output high-pass (Hz)
const ADEC: usize = 9; // amp decay (s)
const AREL: usize = 10; // amp release (s)
const NDEC: usize = 11; // noise decay (s)
const NREL: usize = 12; // noise release (s, before velocity scaling)
const NAMT: usize = 13; // noise amount
const NCOL: usize = 14; // noise filter offset (semitones)

pub(crate) struct Classic {
    /// +0x80 Tune as set; +0xa4 gate; +0xa8 requested / +0xac active core mode.
    p0: f32,
    gate: bool,
    mode_req: u32,
    mode: u32,
    /// +0xb0 note (41…65), +0xb4 1 − grit, +0xb8 base note, +0xbc decay, +0xc0 osc mix,
    /// +0xc4 punch.
    note_f: f32,
    grit_inv: f32,
    base: f32,
    decay: f32,
    mix: f32,
    punch: f32,
    /// +0xcc pitch (Hz), +0xd0 previous noise release, +0xd4 MIDI note, +0xd8 last amp,
    /// +0xdc velocity peak.
    freq: f32,
    nrel_prev: f32,
    midi_note: u32,
    amp_prev: f32,
    vel_peak: f32,
    // Oscillators (+0xe0…+0x10c): phase, increment, 1/sr, phase offset.
    ph1: f32,
    inc1: f32,
    inv1: f32,
    off1: f32,
    ph2: f32,
    inc2: f32,
    inv2: f32,
    off2: f32,
    /// +0x114 noise scale, +0x11c LCG state (never reset: free-running from creation).
    nscale: f32,
    lcg: i32,
    nlp: OnePole,
    svf1: Svf,
    svf2: Svf,
    svf3: Svf,
    hp: OnePole,
    /// +0x220 clip level, +0x224 saturation.
    clip: f32,
    sat: f32,
    dl1: Delay,
    dl2: Delay,
    sh1: Hold,
    sh2: Hold,
    // Pitch envelope (+0x80278 time, value, factor, 1/sr).
    ptime: f32,
    pval: f32,
    pcoef: f32,
    pinv: f32,
    // Bit's 10 ms mix envelope (+0x8028c).
    e3_val: f32,
    e3_coef: f32,
    // Click (+0x802a0): 1 for 1 ms, then a 20 µs half-life decay.
    clk_state: u32,
    clk_att: f32,
    clk_v: f32,
    clk_coef: f32,
    clk_inv: f32,
    // Gate (+0x802b8): released by note-off, 1 ms half-life.
    gate_state: u32,
    gate_v: f32,
    gate_coef: f32,
    env1: FreeEnv,
    env2: FreeEnv,
    // De-click (+0x804c4…).
    dc_ph: f32,
    dc_acc: f32,
    dc_cur: f32,
    dc_out: f32,
    dc_prev: f32,
    dc_slope: f32,
    dc_val: f32,
    dc_step: f32,
    dc_inv: f32,
    // Peak hold of the noise envelope (+0x804ec…).
    pk_last: f32,
    pk_auto: bool,
    pk_dec: f32,
    pk_coef: f32,
    pk_out: f32,
    pk_hold: f32,
    pk_inv: f32,
    pk_n: u32,
    pk_invn: f32,
    pk_cnt: u32,
    pk_flag: bool,
    // Choke (+0x80520): 5 ms half-life.
    chk_state: u32,
    chk_v: f32,
    chk_coef: f32,
    // Parameter smoother (+0x80540).
    cur: [f32; 16],
    step: [f32; 16],
    tgt: [f32; 16],
    sm_start: bool,
    sm_active: bool,
    sm_count: i32,
    sm_n: i32,
    sm_inv: f32,
    sm_state: u32,
    /// +0x80630 velocity, +0x80634 a note started, +0x80638 sample rate.
    vel: f32,
    note_active: bool,
    sr: f32,
    /// The `dirty` template flag: set by note-on, reset and the gate switch.
    dirty: bool,
}

impl Classic {
    /// `ClassicSnareCore()`, `onSetSampleRate(sr, 128)`, `onReset()`.
    pub(crate) fn new(sr: f32) -> Classic {
        let inv = 1.0 / sr;
        let l2 = inv * LN2_NEG;
        let mut c = Classic {
            p0: 0.0,
            gate: false,
            mode_req: 3,
            mode: 3,
            note_f: 0.0,
            grit_inv: 0.0,
            base: 0.0,
            decay: 0.0,
            mix: 0.0,
            punch: 0.0,
            freq: 0.0,
            nrel_prev: 0.0,
            midi_note: 60,
            amp_prev: 0.0,
            vel_peak: 0.0,
            ph1: 0.0,
            inc1: clamp_pm1(inv * 1.0),
            inv1: inv,
            off1: 0.0,
            ph2: 0.0,
            inc2: clamp_pm1(inv * 1.0),
            inv2: inv,
            off2: 0.0,
            nscale: (sr / 44100.0).sqrt() * f32::from_bits(0x3000_009d),
            lcg: 0,
            nlp: OnePole { s: 0.0, a: 0.0, b: 0.0, g: 0.0, cutoff: 9000.0, inv },
            svf1: Svf::new(1.6, sr, inv),
            svf2: Svf::new(1.6, sr, inv),
            svf3: Svf::new(1.4, sr, inv),
            hp: OnePole { s: 0.0, a: 0.0, b: 0.0, g: 0.0, cutoff: 1.0, inv },
            clip: 4.0,
            sat: 0.5,
            dl1: Delay { buf: vec![0.0; 0x10000], w: 0, r: 0, sr },
            dl2: Delay { buf: vec![0.0; 0x10000], w: 0, r: 0, sr },
            sh1: Hold { phase: 0.0, inc: 0.0, held: 0.0, inv },
            sh2: Hold { phase: 0.0, inc: 0.0, held: 0.0, inv },
            ptime: 1.0,
            pval: 0.0,
            pcoef: 0.0,
            pinv: inv,
            e3_val: 0.0,
            e3_coef: 0.0,
            clk_state: 0,
            clk_att: 0.001,
            clk_v: 0.0,
            clk_coef: min1((l2 / 2e-5).exp()),
            clk_inv: inv,
            gate_state: 0,
            gate_v: 0.0,
            gate_coef: min1((l2 / 0.001).exp()),
            env1: FreeEnv::new(true),
            env2: FreeEnv::new(false),
            dc_ph: 1.0,
            dc_acc: 0.0,
            dc_cur: 0.0,
            dc_out: 0.0,
            dc_prev: 0.0,
            dc_slope: 0.0,
            dc_val: 0.0,
            dc_step: (inv * 1.0) * 10.0,
            dc_inv: inv,
            pk_last: 1.0,
            pk_auto: true,
            pk_dec: 0.0,
            pk_coef: 0.0,
            pk_out: 0.0,
            pk_hold: 0.0,
            pk_inv: inv,
            pk_n: 1,
            pk_invn: 1.0,
            pk_cnt: 0,
            pk_flag: false,
            chk_state: 0,
            chk_v: 0.0,
            chk_coef: min1((l2 / 0.005).exp()),
            cur: [0.0; 16],
            step: [0.0; 16],
            tgt: [0.0; 16],
            sm_start: true,
            sm_active: false,
            sm_count: 0,
            sm_n: 1,
            sm_inv: 1.0,
            sm_state: 1,
            vel: 0.0,
            note_active: false,
            sr,
            dirty: true,
        };
        // Smoother: max(round(0.02·sr/128), 1) control steps.
        let n = ((sr * f32::from_bits(0x3ca3_d70a)) / 128.0 + 0.5) as i32;
        c.sm_n = if n > 1 { n } else { 1 };
        c.sm_inv = 1.0 / (c.sm_n * 128) as f32;
        // Noise low-pass at 9 kHz (9000π = 0x46dce4ab).
        let g = tan_g(inv * f32::from_bits(0x46dc_e4ab));
        c.nlp.g = g;
        c.nlp.a = 1.0 / (g + 1.0);
        c.nlp.b = g + -1.0;
        c.svf1.set(1.0);
        c.svf2.set(1.0);
        c.svf3.set(1.0);
        c.hp.set(1.0);
        // Pitch envelope factor for the initial 1 s, and Bit's 10 ms mix envelope.
        c.pcoef = clamp01(exp_e((l2 / (1.0 + TINY_T)).mul_add(DECAY_K, 1200.0)));
        c.e3_coef = clamp01(exp_e((l2 / 0.01).mul_add(DECAY_K, 1200.0)));
        c.env1.set_samplerate(sr);
        c.env1.set_attack(1e-4);
        c.env2.set_samplerate(sr);
        c.env2.set_attack(0.003);
        let n = (sr * f32::from_bits(0x3c75_c290)) as u32;
        c.pk_n = if n > 1 { n } else { 1 };
        c.pk_invn = 1.0 / c.pk_n as f32;
        c.reset();
        c
    }

    /// `onReset()` (also what `processAudio` does once after the sound has ended).
    fn reset(&mut self) {
        self.vel = 0.0;
        self.nrel_prev = 0.0;
        self.note_active = false;
        self.amp_prev = 0.0;
        self.vel_peak = 0.0;
        self.cur = self.tgt;
        self.sm_start = true;
        self.sm_state = 1;
        self.ph1 = 0.0;
        self.off1 = 0.0;
        self.ph2 = 0.0;
        self.off2 = 0.0;
        self.nlp.s = 0.0;
        for f in [&mut self.svf1, &mut self.svf2, &mut self.svf3] {
            f.ic1 = 0.0;
            f.ic2 = 0.0;
        }
        self.hp.s = 0.0;
        self.dl1.buf.fill(0.0);
        self.dl2.buf.fill(0.0);
        self.sh1.phase = 0.0;
        self.sh2.phase = 0.0;
        self.pval = 0.0;
        self.e3_val = 0.0;
        self.clk_state = 0;
        self.clk_v = 0.0;
        self.gate_state = 0;
        self.gate_v = 0.0;
        for e in [&mut self.env1, &mut self.env2] {
            e.stage = 0;
            e.out = 0.0;
            e.x = 1.0;
            e.t = 0.0;
        }
        self.chk_v = 0.0;
        self.dc_ph = 1.0;
        self.dc_acc = 0.0;
        self.dc_cur = 0.0;
        self.dc_out = 0.0;
        self.dc_prev = 0.0;
        self.dc_slope = 0.0;
        self.dc_val = 0.0;
        self.pk_out = 0.0;
        self.pk_hold = 0.0;
        self.pk_dec = 0.0;
        self.pk_flag = false;
        self.pk_cnt = 0;
        self.chk_state = 0;
        self.dirty = true;
    }

    /// A target changed: start a ramp at the next control step.
    fn mark(&mut self) {
        self.sm_start = true;
        self.sm_state = 1;
    }

    /// `setParamTune(v)`: note, pitch envelope, base note, high-pass, decays, grit notes.
    fn set_tune(&mut self, v: f32) {
        let n = clamp01(v).mul_add(24.0, 41.0) + (self.midi_note as f32 + -60.0);
        let n = if n < 41.0 { 41.0 } else { n };
        let n = if n > 65.0 { 65.0 } else { n };
        self.note_f = n;
        self.set_pitch_env();
        self.base = if self.mode > 0 { n + -0.5 } else { n };
        self.tgt[BASE] = self.base;
        self.tgt[HP_HZ] = hz(self.base + -5.0);
        self.mark();
        self.calc_decay();
        self.set_grit_notes();
        self.mark();
    }

    /// Pitch-envelope time and depth: depth 0 (Tonal), 22·punch (Punchy), 11 (Bit).
    fn set_pitch_env(&mut self) {
        let a = 21.0 - (self.note_f + -52.0);
        let a = if a > 20.0 { 20.0 } else { a };
        let (off, depth) = match self.mode {
            0 => (0.0, 0.0),
            1 => {
                let p = clamp01(self.punch);
                (p.mul_add(10.0, -5.0), p.mul_add(22.0, 0.0))
            }
            _ => (0.0, 11.0),
        };
        self.tgt[PDEPTH] = depth;
        self.tgt[PTIME] = db_time(a + off);
    }

    fn set_grit_notes(&mut self) {
        let g = clamp01(self.grit_inv);
        self.tgt[SH_NOTE] = g.mul_add(42.0, 30.0) + self.base;
        self.tgt[F3_NOTE] = g.mul_add(35.0, 40.0) + self.base;
    }

    fn set_mix(&mut self) {
        if self.mode > 0 {
            self.tgt[OSC2] = 1.0;
            self.tgt[OSC1] = 0.0;
        } else {
            self.tgt[OSC2] = 1.0 - self.mix;
            self.tgt[OSC1] = self.mix;
        }
    }

    /// `calculateDecayParams()`: amp decay/release and noise decay/release times.
    fn calc_decay(&mut self) {
        let a = (self.note_f + -52.0) * f32::from_bits(0xbfd5_5558);
        let f1 = a * f32::from_bits(0x3e19_999a);
        let f4 = a * 0.5;
        let d = clamp01(self.decay);
        let s7 = d.mul_add(21.0, 25.0);
        let s3 = d.mul_add(25.0, 0.0);
        let s5: f32 = if self.mode > 0 { 28.0 } else { 23.0 };
        let s4 = f4 + s7;
        let v0 = db_time(s4 + -5.0);
        let s5 = s5.mul_add(d, 24.0);
        let s3 = f1 + s3;
        let v2 = db_time(s4);
        let s4b = f1 + s5;
        let v1 = db_time(s3);
        let v3 = db_time(s4b);
        self.tgt[ADEC] = v0;
        self.tgt[AREL] = if self.mode > 1 { v2 * f32::from_bits(0x3e4c_cccd) } else { v2 };
        self.tgt[NDEC] = v1;
        self.tgt[NREL] = v3;
        self.mark();
    }

    /// `processControlRate()` → `SmoothingToolbox::startSmooth()` and the ramp counter.
    fn smooth_control(&mut self) {
        if self.sm_start {
            self.sm_start = false;
            self.sm_state = 2;
            let mut any = false;
            for i in 0..16 {
                let s = (self.tgt[i] - self.cur[i]) * self.sm_inv;
                if ((s * s) as f64) < 1e-15 {
                    self.step[i] = 0.0;
                    self.cur[i] = self.tgt[i];
                } else {
                    self.step[i] = s;
                    any = true;
                }
            }
            if any {
                self.sm_count = self.sm_n;
                self.sm_active = true;
            } else {
                self.sm_state = 4;
            }
            return;
        }
        self.sm_count = self.sm_count.wrapping_sub(1);
        if self.sm_count == 0 {
            self.cur = self.tgt;
            self.step = [0.0; 16];
            self.sm_active = false;
            self.sm_state = 4;
        } else if self.sm_state == 2 {
            self.sm_state = 3;
        } else if self.sm_state == 4 {
            self.sm_state = 0;
        }
    }

    /// The peak-hold decay factor for time `t` (s): `2^(−3/(sr·t))`, ≤ 0.99999.
    fn peak_coef(&self, t: f32) -> f32 {
        let lo = self.pk_inv * -3.0;
        let hi = self.pk_inv * f32::from_bits(0x4a00_2c80);
        let t = if t < 1e-4 { 1e-4 } else { t };
        let t = if hi < t { hi } else { t };
        let r = fast_exp2(lo / t);
        let m = f32::from_bits(0x3f7f_ff58);
        if r > m { m } else { r }
    }

    /// `setNoteOn`'s amp-envelope release setup: exponential (Volt) or linear (Bit).
    fn set_amp_release_kind(&mut self) {
        let e = &mut self.env1;
        if self.mode <= 1 {
            e.rel_shape = -1.0;
            let c = (LN8 / (e.rel_raw * e.sr as f64)).exp();
            e.rel_coef = c;
            if e.stage == 4 {
                e.mul = c;
            }
        } else {
            e.rel_shape = 0.0;
            e.rel_coef = 0.0;
            e.rel_time = (e.rel_raw + e.rel_raw) + TINY;
            e.inv_rel = 1.0 / e.rel_time;
            if e.stage == 4 {
                e.sub = e.inv_rel * e.inv_sr;
            }
        }
    }

    /// A one-pole "AR" helper envelope's decay step (`state` 2): `v·coef` until < 1e-4.
    #[inline]
    fn decay_step(state: &mut u32, v: &mut f32, coef: f32) -> f32 {
        let x = coef * *v;
        *v = x;
        if (x as f64) < END {
            *v = 0.0;
            *state = 0;
            0.0
        } else {
            x
        }
    }

    /// One sample of `processAudioImpl<bit, gate, ramp, dirty>`.
    #[inline]
    fn sample(&mut self, upd: Upd, bit: bool) -> f32 {
        if self.sm_active {
            for i in 0..16 {
                self.cur[i] += self.step[i];
            }
        }
        let st = self.step;
        let cur = self.cur;
        let chg = |i: usize| match upd {
            Upd::Full => true,
            Upd::Ramp => st[i] != 0.0,
            Upd::Skip => false,
        };
        // Pitch envelope.
        if chg(PTIME) && self.ptime != cur[PTIME] {
            self.ptime = cur[PTIME];
            let x = (self.pinv * LN2_NEG) / (cur[PTIME] + TINY_T);
            self.pcoef = clamp01(exp_e(x.mul_add(DECAY_K, 1200.0)));
        }
        let pitch = match upd {
            Upd::Full => true,
            Upd::Ramp => !(st[BASE] == 0.0 && st[PDEPTH] == 0.0 && self.pval < THR),
            Upd::Skip => self.pval >= THR,
        };
        if pitch {
            let v = self.pcoef * self.pval;
            self.pval = v;
            let e = if v < THR {
                self.pval = 0.0;
                0.0
            } else {
                v - THR
            };
            let f = hz(e.mul_add(cur[PDEPTH], cur[BASE]));
            self.freq = f;
            self.inc1 = clamp_pm1((f * f32::from_bits(0x3fe0_2a45)) * self.inv1);
            self.inc2 = clamp_pm1(f * self.inv2);
        }
        self.ph1 = wrap(self.ph1 + self.inc1 + 10.0);
        let tri1 = (wrap(self.off1 + self.ph1 + 10.0) * 2.0).abs() + -0.5;
        self.ph2 = wrap(self.inc2 + self.ph2 + 10.0);
        let tri2 = (wrap(self.off2 + self.ph2 + 10.0) * 2.0).abs() + -0.5;
        // Amp envelope.
        if chg(ADEC) {
            self.env1.set_decay(cur[ADEC]);
        }
        if chg(AREL) {
            self.env1.set_release(cur[AREL]);
        }
        let amp = self.env1.process() * self.vel_peak;
        self.amp_prev = amp;
        // Oscillators and click.
        let tp = f32::from_bits(0x3e22_f981);
        let s1 = sine((tri1 * PI) * tp);
        let s2 = sine((tri2 * PI) * tp);
        let clk = match self.clk_state {
            0 => 0.0,
            1 => {
                let v = self.clk_inv + self.clk_v;
                self.clk_v = v;
                if v > self.clk_att {
                    self.clk_v = 1.0;
                    self.clk_state = 2;
                }
                1.0
            }
            _ => Self::decay_step(&mut self.clk_state, &mut self.clk_v, self.clk_coef),
        };
        let o1 = amp * (amp * (cur[OSC1] * s1));
        let vel = self.vel;
        let o2 = (vel * f32::from_bits(0x3fb3_3333) - s2).mul_add(clamp01(clk), s2);
        let mut osc = amp * (o1 + cur[OSC2] * o2);
        // De-click.
        self.dc_prev = self.dc_out;
        self.dc_cur = osc;
        if self.dc_ph > 0.0 {
            let p = self.dc_ph - self.dc_step;
            let h = p + -0.5;
            let h = (-h).mul_add(h.abs(), h);
            let w = h.mul_add(2.0, 0.5);
            let add = (self.dc_val + self.dc_acc) * w;
            self.dc_ph = p;
            self.dc_acc += self.dc_slope;
            osc += add;
        }
        self.dc_out = osc;
        // Choke.
        let chk = match self.chk_state {
            0 => 0.0,
            1 => 1.0,
            _ => Self::decay_step(&mut self.chk_state, &mut self.chk_v, self.chk_coef),
        };
        // Noise.
        self.lcg = self.lcg.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        let x = self.nscale * self.lcg as f32;
        let s = self.nlp.s;
        let y = self.nlp.step(x);
        let noise = self.nlp.g * (s + y);
        let cn = (cur[BASE] + 53.0) + cur[NCOL];
        if chg(NCOL) {
            let fc = hz(cn);
            self.svf1.update(fc);
            self.svf2.update(fc);
        }
        let (_, bp1) = self.svf1.tick(noise);
        let (hp2, bp2) = self.svf2.tick(bp1);
        let srfac = clamp01((self.sr + -88200.0) / -44100.0);
        let colfac = clamp01((cn + -77.0) / 28.0);
        // Noise envelope with peak hold.
        if chg(NDEC) {
            self.env2.set_decay(cur[NDEC]);
        }
        let k07 = f32::from_bits(0x3f33_3333);
        self.env2.set_release(clamp01(vel).mul_add(k07, k07) * cur[NREL]);
        let e2 = self.env2.process();
        if self.pk_last != self.nrel_prev {
            self.pk_last = self.nrel_prev;
            if self.pk_auto {
                self.pk_coef = self.peak_coef(self.nrel_prev);
            }
        }
        let ne = vel * e2;
        self.nrel_prev = cur[NREL];
        let a = self.pk_dec * self.pk_hold;
        self.pk_dec *= self.pk_coef;
        let m = if a < ne { ne } else { a };
        let r = ((ne + a) - m).mul_add(0.5, m);
        let r = if r < 0.0 { 0.0 } else { r };
        self.pk_out = r;
        let mut fade = 1.0;
        if r < THR {
            if self.pk_cnt < self.pk_n {
                self.pk_cnt += 1;
            } else {
                self.pk_flag = false;
                self.pk_cnt = self.pk_n;
            }
            let f = (-(self.pk_cnt as f32)).mul_add(self.pk_invn, 1.0);
            fade = f * (f * f);
        }
        let env = r * fade;
        let n = (bp2 + colfac * ((hp2 * f32::from_bits(0x3ecc_cccd)) * srfac)) * 20.0;
        let n = n * env;
        let n = if n < -self.clip { -self.clip } else { n };
        let n = if n > self.clip { self.clip } else { n };
        let nsat = n * (n.abs() * -0.25).mul_add(self.sat, 1.0);
        // Gate.
        let gate = match (self.gate, self.gate_state) {
            (false, _) => 1.0,
            (_, 0) => 0.0,
            (_, 1) => 1.0,
            _ => Self::decay_step(&mut self.gate_state, &mut self.gate_v, self.gate_coef),
        };
        let (sum, k) = if !bit {
            let vf = clamp01(vel).mul_add(f32::from_bits(0x3eb3_3334), f32::from_bits(0x3f26_6666));
            (osc + cur[NAMT] * (vf * nsat), f32::from_bits(0x3f0c_d6a1))
        } else {
            let x = osc + nsat;
            let mut dz = 0.0;
            let t = f32::from_bits(0x3dcc_cccd);
            if x.abs() > t {
                let lo = x + t;
                let lo = if lo > 0.0 { 0.0 } else { lo };
                let hi = x + -t;
                let hi = if hi < 0.0 { 0.0 } else { hi };
                let d = if x <= 0.0 { lo } else { hi };
                let lim = if env > 0.5 { 0.5 } else { env };
                let d = if d < -lim { -lim } else { d };
                let d = if d > lim { lim } else { d };
                dz = d * f32::from_bits(0x3f4c_cccd);
            }
            if chg(DTIME) {
                self.dl1.set(cur[DTIME]);
                self.dl2.set(cur[DTIME]);
            }
            if chg(SH_NOTE) {
                self.sh1.inc = hz(cur[SH_NOTE]) * self.sh1.inv;
                self.sh2.inc = hz(cur[SH_NOTE] + 2.0) * self.sh2.inv;
            }
            let comb = dz + self.dl1.tick(dz);
            let sh = self.sh1.tick(comb);
            let d2 = self.dl2.tick(sh);
            let v = self.e3_coef * self.e3_val;
            self.e3_val = v;
            let e3 = if v < THR {
                self.e3_val = 0.0;
                0.0
            } else {
                v - THR
            };
            let m3 = clamp01(e3.mul_add(-0.5, 1.0));
            if chg(F3_NOTE) {
                self.svf3.update(hz(cur[F3_NOTE]));
            }
            let y = ((d2 + sh) - x).mul_add(m3, x) * 4.0;
            let (_, bp3) = self.svf3.tick(y);
            let osc_sh = self.sh2.tick(osc);
            (((bp3 * 0.75) * cur[NAMT]) + osc_sh, f32::from_bits(0x3ef2_8566))
        };
        let sum = if self.gate { sum * gate } else { sum };
        if chg(HP_HZ) && self.hp.cutoff != cur[HP_HZ] {
            self.hp.set(cur[HP_HZ]);
        }
        let s = self.hp.s;
        let y = self.hp.step(sum);
        chk * ((y - s) * k)
    }
}

impl Core for Classic {
    /// `setParameter(idx, v)`.
    fn set_parameter(&mut self, idx: u32, v: f32) {
        match idx {
            0 => {
                self.p0 = v;
                self.set_tune(v);
            }
            1 => {
                self.decay = v;
                self.calc_decay();
            }
            2 => self.mode_req = v as u32,
            3 => {
                self.mix = v;
                self.set_mix();
                self.mark();
            }
            4 => {
                self.punch = v;
                self.set_pitch_env();
                self.mark();
            }
            5 => {
                self.grit_inv = 1.0 - v;
                self.set_grit_notes();
                self.mark();
            }
            6 => {
                let c = clamp01(v);
                self.tgt[NCOL] = c.mul_add(48.0, -24.0);
                let e = c.mul_add(16.0, 45.0) + 40.5;
                self.tgt[DTIME] = 1.0 / hz(e);
                self.mark();
            }
            7 => {
                self.tgt[NAMT] = v;
                self.mark();
            }
            8 => {
                self.gate = v > 0.0;
                self.dirty = true;
            }
            _ => {}
        }
    }

    /// `setNoteOn(velocity, note)`.
    fn note_on(&mut self, v: f32, note: u32) {
        if !(0.0..=1.0).contains(&v) {
            self.vel = 0.0;
            return;
        }
        self.midi_note = note;
        self.set_tune(self.p0);
        if self.mode != self.mode_req {
            self.mode = self.mode_req;
            self.set_tune(self.p0);
            self.set_mix();
        }
        self.cur = self.tgt;
        self.mark();
        self.ph1 = 0.25;
        self.ph2 = f32::from_bits(0x3ecc_cccd);
        self.pval = 1.0;
        self.e3_val = 1.0;
        self.clk_state = 1;
        self.clk_v = 0.0;
        self.set_amp_release_kind();
        self.env1.note_on(v);
        self.env2.note_on(v);
        self.dc_step = (self.freq * self.dc_inv) * 10.0;
        self.dc_ph = 1.0;
        self.dc_acc = 0.0;
        self.dc_slope = self.dc_out - self.dc_prev;
        self.dc_val = self.dc_cur;
        self.gate_state = 1;
        self.gate_v = 0.0;
        self.chk_state = 1;
        self.chk_v = 0.0;
        self.pk_dec = 1.0;
        self.pk_hold = self.pk_out;
        if !self.pk_auto {
            self.pk_coef = self.peak_coef(self.pk_last);
        }
        self.pk_flag = true;
        self.pk_cnt = 0;
        self.vel_peak = if self.amp_prev > v { self.amp_prev } else { v };
        self.vel = v;
        self.note_active = true;
        self.dirty = true;
    }

    /// `setNoteOff`: releases the gate.
    fn note_off(&mut self, _note: u32) {
        if self.gate_state == 1 {
            self.gate_state = 2;
            self.gate_v = 1.0;
        }
    }

    /// `onChoke`: 5 ms fade.
    fn choke(&mut self) {
        if self.chk_state == 1 {
            self.chk_state = 2;
            self.chk_v = 1.0;
        }
    }

    fn leave(&mut self) {
        self.choke();
    }

    fn control_rate(&mut self) {
        self.smooth_control();
    }

    /// `processAudio`: the sample loop while sounding; once silent, `onReset` (once).
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        if !self.has_output() {
            if self.note_active {
                self.reset();
            }
            l.fill(0.0);
            r.fill(0.0);
            return;
        }
        let upd = if self.dirty {
            Upd::Full
        } else if self.sm_active {
            Upd::Ramp
        } else {
            Upd::Skip
        };
        let bit = self.mode == 2;
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            let y = self.sample(upd, bit);
            *a = y;
            *b = y;
        }
        self.dirty = false;
    }

    /// `hasOutputSignal()`.
    fn has_output(&self) -> bool {
        (self.env1.stage != 0 || self.env2.stage != 0) && self.chk_state != 0 && (!self.gate || self.gate_state != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(c: &mut Classic, n: usize) -> Vec<f32> {
        let mut out = vec![0.0; n];
        let mut r = [0.0f32; 16];
        let mut at = 0;
        while at < n {
            if at % 128 == 0 {
                c.control_rate();
            }
            let k = (n - at).min(16);
            c.process(&mut out[at..at + k], &mut r[..k]);
            at += k;
        }
        out
    }

    fn setup(mode: f32) -> Classic {
        let mut c = Classic::new(48_000.0);
        for (i, v) in [0.5, 0.5, mode, 0.5, 0.25, 0.3, 0.5, 0.25, 0.0].iter().enumerate() {
            c.set_parameter(i as u32, *v);
        }
        c
    }

    #[test]
    fn every_mode_renders_and_ends() {
        for mode in [0.0, 1.0, 2.0] {
            let mut c = setup(mode);
            c.note_on(0.8, 60);
            let y = run(&mut c, 48_000 * 4);
            assert!(y.iter().all(|x| x.is_finite()), "mode {mode}");
            let peak = y.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak > 0.05, "mode {mode}: peak {peak}");
            assert!(!c.has_output(), "mode {mode}: still sounding");
            assert!(y[y.len() - 1000..].iter().all(|&x| x == 0.0));
        }
    }

    #[test]
    fn gate_releases_on_note_off() {
        let mut c = setup(0.0);
        c.set_parameter(8, 1.0);
        c.note_on(1.0, 60);
        run(&mut c, 4800);
        assert!(c.has_output());
        c.note_off(60);
        run(&mut c, 4800);
        assert!(!c.has_output());
    }

    #[test]
    fn param_change_mid_note_keeps_the_voice() {
        let mut c = setup(0.0);
        c.note_on(1.0, 60);
        let a = run(&mut c, 2400);
        c.set_parameter(0, 0.9);
        c.set_parameter(1, 0.1);
        let b = run(&mut c, 2400);
        assert!(c.has_output());
        let jump = (b[0] - a[a.len() - 1]).abs();
        assert!(jump < 0.2, "jump {jump}");
        assert!(b.iter().all(|x| x.is_finite()));
    }
}
