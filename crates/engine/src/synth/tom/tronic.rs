//! Tom's Tronic engine: `DSP::DustyTomCore` (docs/synth-tom.md §3). Three triangle
//! oscillators (the note, the note plus Interval, and a partial 17.51 semitones up)
//! with a pitch envelope (Bend), soft clipping, an FM'd triangle sweeping the phases, a
//! filtered noise burst, four `FreeEnvelope`s and a DC blocker. Ported operation by
//! operation from the arm64 code; offsets in the comments are `DustyTomCore` member
//! offsets.

use super::Engine;
use super::dsp::{clamp01, db_ms, exp_e, frecpe, pitch_hz};

#[inline]
const fn c(b: u32) -> f32 {
    f32::from_bits(b)
}

/// `-ln 2` and 86.5617 (= 60·log2 e · ln 2 … the exp-table scale for decays).
const LN2_NEG: f32 = c(0xbf31_7215);
const DECAY_K: f32 = c(0x42ad_1f97);
/// 1e-30.
const TINY: f32 = c(0x0da2_4260);
/// −ln 8: the FreeEnvelope's exponential segments fall to 1/8 in their time.
const LN8_NEG: f64 = f64::from_bits(0xc000_a2b1_704f_f434);
/// 0.0001 and 1/(1 − 0.0001): a segment ends at 0.0001.
const END: f64 = f64::from_bits(0x3f1a_36e2_eb1c_432d);
const END_SCALE: f64 = f64::from_bits(0x3ff0_0068_de3a_efe5);
/// "Never" (1e30).
const NEVER: f64 = f64::from_bits(0x4629_3e59_39a0_8cea);
/// 1e-20.
const TINY_D: f64 = f64::from_bits(0x3bc7_9ca1_0000_0000);
/// π.
const PI: f32 = c(0x4049_0fdb);

/// `clamp(x, −k, k)`.
#[inline]
fn clamp_sym(x: f32, k: f32) -> f32 {
    let a = if x < -k { -k } else { x };
    if a > k { k } else { a }
}

/// `x·(1 − |x|·k/4)`: the soft saturation on envelope outputs.
#[inline]
fn soft(x: f32, k: f32) -> f32 {
    x * (x.abs() * -0.25).mul_add(k, 1.0)
}

/// `g ≈ tan(w)` from its Taylor terms `w + w³/3 + 2w⁵/15` (as compiled).
#[inline]
fn tan_approx(w: f32) -> f32 {
    let w2 = w * w;
    let w3 = w * w2;
    let w5 = w2 * w3;
    w + w5.mul_add(c(0x3e08_8886), w3 * c(0x3eaa_aaaa))
}

/// `trunc(x)` as the vector code computes it (`x` itself from 2^24 up).
#[inline]
fn trunc24(x: f32) -> f32 {
    if x.abs() < 16_777_216.0 { x as i32 as f32 } else { x }
}

/// `1/x` as `frecpe` and two `frecps` Newton steps.
#[inline]
fn recip(x: f32) -> f32 {
    let r0 = frecpe(x);
    let r1 = r0 * (-x).mul_add(r0, 2.0);
    (-x).mul_add(r1, 2.0) * r1
}

/// The vector `2^x`: rounded to the nearest integer (halves away from zero), a cubic
/// for the remainder, the integer put into the exponent.
#[inline]
fn exp2_poly(x: f32) -> f32 {
    let a = x.abs();
    let mut t = if a < 16_777_216.0 { a as i32 as f32 } else { a };
    if a > t {
        t += 1.0;
    }
    if t + -0.5 > a {
        t += -1.0;
    }
    let r = if a > 16_777_216.0 { x } else { t.abs().copysign(x) };
    let n = r as i32;
    let f = x - n as f32;
    let scale = f32::from_bits((n << 23).wrapping_add(0x3f80_0000) as u32);
    let p = f * c(0x3d65_6042) + c(0x3e77_ced9);
    let p = f * p + c(0x3f31_6873);
    let p = f * p + 1.0;
    p * scale
}

/// A one-pole section (+0x198, +0x1b8, +0x1d8, +0x228): state, `1/(g+1)`, `g − 1`, `g`.
#[derive(Clone, Copy, Default)]
struct OnePole {
    s: f32,
    a: f32,
    b: f32,
    g: f32,
}

impl OnePole {
    /// The coefficients for `w = π·f/sr`.
    fn tune(&mut self, w: f32) {
        let g = tan_approx(w);
        self.g = g;
        self.a = 1.0 / (g + 1.0);
        self.b = g + -1.0;
    }

    /// The low-pass output `g·(s + v)` for input `x`.
    #[inline]
    fn lowpass(&mut self, x: f32) -> f32 {
        let v = self.a * (-self.b).mul_add(self.s, x);
        let y = self.g * (self.s + v);
        self.s = v;
        y
    }

    /// The high-pass output `v − s` for input `x`.
    #[inline]
    fn highpass(&mut self, x: f32) -> f32 {
        let v = self.a * (-self.b).mul_add(self.s, x);
        let y = v - self.s;
        self.s = v;
        y
    }
}

/// `UTIL::FreeEnvelope<off, breakpoint, …, log, R>`: an attack (straight or shaped), an
/// exponential decay to a sustain level and a release, stepped in `f64`. Offsets are
/// relative to the envelope (docs/synth-tom.md §3).
#[derive(Clone)]
struct FreeEnv {
    /// +0x10: 0 idle, 1 attack, 2 decay, 4 release.
    stage: i32,
    /// +0x14: the level.
    level: f32,
    /// +0x18 the segment's running value (1 → 0), +0x20 its time, +0x28 its time limit,
    /// +0x30 per-frame decrement, +0x38 per-frame factor.
    x: f64,
    t: f64,
    t_end: f64,
    dec: f64,
    mul: f64,
    /// +0x40 decay factor, +0x48 exponential release factor.
    dcoef: f64,
    rcoef: f64,
    /// +0x50 the level the release started from.
    rel_start: f32,
    /// +0x58 attack, +0x60 1/attack, +0x68 decay, +0x70 1/decay, +0x78 release (scaled
    /// by the shape), +0x80 release, +0x88 1/scaled release.
    att: f64,
    att_inv: f64,
    dec_t: f64,
    dec_inv: f64,
    rel_t: f64,
    rel_t2: f64,
    rel_inv: f64,
    /// +0x90/+0x98/+0xa0 segment shapes (0 straight; release −1 = exponential).
    att_shape: f64,
    dec_shape: f64,
    rel_shape: f64,
    /// +0xa8 sustain, +0xb4 gain, +0xb8 velocity sensitivity, +0xbc velocity factor.
    sustain: f32,
    gain: f32,
    vel_sens: f32,
    vel: f32,
    /// +0xe0/+0xe4/+0xe8 the times last set, +0xec the rate, +0xf0 1/rate.
    att_f: f32,
    dec_f: f32,
    rel_f: f32,
    sr_f: f32,
    inv_sr: f64,
    /// Template `FreeEnvelopeRelease` = 1: the release follows its shape.
    shaped_release: bool,
}

impl FreeEnv {
    fn new(att_shape: f64, shaped_release: bool) -> FreeEnv {
        FreeEnv {
            stage: 0,
            level: 0.0,
            x: 1.0,
            t: 0.0,
            t_end: 0.0,
            dec: 0.0,
            mul: 0.0,
            dcoef: 0.0,
            rcoef: 0.0,
            rel_start: 0.0,
            att: 1.0,
            att_inv: 1.0,
            dec_t: 1.0,
            dec_inv: 1.0,
            rel_t: 2.0,
            rel_t2: 1.0,
            rel_inv: 0.5,
            att_shape,
            dec_shape: 0.0,
            rel_shape: 0.0,
            sustain: 0.0,
            gain: 1.0,
            vel_sens: 0.0,
            vel: 1.0,
            att_f: 0.0,
            dec_f: 0.0,
            rel_f: 0.0,
            sr_f: -1.0,
            inv_sr: 1.0,
            shaped_release,
        }
    }

    /// The decay factor `exp(−ln 8/(k·t·sr))`, `k` from the decay shape.
    fn decay_coef(&self, dec_t: f64) -> f64 {
        let s = self.dec_shape;
        let k = if s <= 0.0 {
            s + 1.0
        } else {
            let d = 1.0 - s;
            if d < 1e-7 { 1e7 } else { 1.0 / d }
        };
        (LN8_NEG / (k * dec_t * self.sr_f as f64)).exp()
    }

    /// `setSamplerate`: times re-clamped to the rate; the envelope idles at 0.
    fn set_sample_rate(&mut self, sr: f32) {
        self.sr_f = sr;
        let srd = sr as f64;
        self.inv_sr = 1.0 / srd;
        self.stage = 0;
        self.level = 0.0;
        self.x = 1.0;
        self.t = 0.0;
        let lo = self.inv_sr as f32;
        let att = self.att as f32;
        let a = if lo > att { lo } else { att };
        let a = if a > 100.0 { 100.0 } else { a };
        self.att = a as f64;
        self.att_inv = 1.0 / self.att;
        let dec = self.dec_t as f32;
        self.att_f = att;
        self.dec_f = dec;
        let d = if lo > dec { lo } else { dec };
        let d = if d > 100.0 { 100.0 } else { d };
        self.dec_t = d as f64;
        self.dec_inv = 1.0 / self.dec_t;
        self.dcoef = self.decay_coef(self.dec_t);
        let rel = self.rel_t as f32;
        self.rel_f = rel;
        self.rel_t2 = rel as f64;
        if self.rel_shape == -1.0 {
            self.rcoef = (LN8_NEG / (srd * self.rel_t2)).exp();
        } else {
            self.rcoef = 0.0;
            let s = (-self.rel_shape).mul_add(self.rel_shape, 1.0);
            self.rel_t = (self.rel_t2 + self.rel_t2) / (s * s) + TINY_D;
            self.rel_inv = 1.0 / self.rel_t;
        }
    }

    /// `setAttack(t)` (not before a rate is set).
    fn set_attack(&mut self, t: f32) {
        if self.sr_f <= 0.0 || self.att_f == t {
            return;
        }
        self.att_f = t;
        let lo = self.inv_sr as f32;
        let a = if t < lo { lo } else { t };
        let a = if a > 100.0 { 100.0 } else { a };
        self.att = a as f64;
        self.att_inv = 1.0 / self.att;
    }

    /// `setDecay(t)`, as `processAudio` updates it every frame.
    fn set_decay(&mut self, t: f32) {
        if self.dec_f == t {
            return;
        }
        self.dec_f = t;
        let lo = self.inv_sr as f32;
        let d = if t < lo { lo } else { t };
        let d = if d > 100.0 { 100.0 } else { d };
        let dd = d as f64;
        self.dec_t = dd;
        self.dec_inv = 1.0 / dd;
        let k = self.decay_coef(dd);
        self.dcoef = k;
        if self.stage == 2 {
            self.dec = 0.0;
            self.mul = k;
            self.t_end = if self.sustain != 0.0 { dd } else { NEVER };
        }
    }

    /// `setRelease(0)`.
    fn set_release_zero(&mut self) {
        if self.rel_f == 0.0 {
            return;
        }
        self.rel_f = 0.0;
        self.rel_t2 = 0.0;
        if self.rel_shape == -1.0 {
            let k = (LN8_NEG / (self.sr_f as f64 * 0.0)).exp();
            self.rcoef = k;
            if self.stage == 4 {
                self.mul = k;
            }
        } else {
            self.rcoef = 0.0;
            let s = (-self.rel_shape).mul_add(self.rel_shape, 1.0);
            self.rel_t = 0.0 / (s * s) + TINY_D;
            self.rel_inv = 1.0 / self.rel_t;
            if self.stage == 4 {
                self.dec = self.rel_inv * self.inv_sr;
            }
        }
    }

    /// Idle at level 0 (`onReset`).
    fn clear(&mut self) {
        self.stage = 0;
        self.level = 0.0;
        self.x = 1.0;
        self.t = 0.0;
    }

    fn note_on(&mut self, vel: f32) {
        if !(0.0..1.0001).contains(&vel) {
            return;
        }
        self.clear();
        self.advance();
        self.vel = self.vel_sens.mul_add(vel * vel, 1.0 - self.vel_sens);
    }

    /// `advanceStage`.
    fn advance(&mut self) {
        self.x = 1.0;
        self.t = 0.0;
        match self.stage {
            0 => {
                self.mul = 1.0;
                self.stage = 1;
                self.t_end = self.att;
                self.dec = self.inv_sr * self.att_inv;
            }
            1 => {
                self.dec = 0.0;
                self.mul = self.dcoef;
                self.stage = 2;
                self.t_end = if self.sustain == 0.0 { NEVER } else { self.dec_t };
            }
            2 | 3 => {
                self.rel_start = self.level;
                if self.rel_shape == -1.0 {
                    self.dec = 0.0;
                    self.mul = self.rcoef;
                } else {
                    self.dec = self.inv_sr * self.rel_inv;
                    self.mul = 1.0;
                }
                self.t_end = NEVER;
                self.stage = 4;
            }
            4 => {
                self.dec = 0.0;
                self.mul = 1.0;
                self.stage = 0;
            }
            _ => {}
        }
    }

    /// The shaped curve (attack, shaped release).
    fn curve(u: f64, shape: f64) -> f64 {
        let lim = f64::from_bits(0x3fef_7ced_9168_72b0);
        let a = shape.abs();
        let a = if a > lim { lim } else { a };
        let d6 = (a + 1.0) * (a + 1.0) + -1.0;
        let d7 = 1.0 / d6;
        let w = 1.0 - a;
        let d16 = w * w;
        let r = 1.0 / w;
        let r2 = r * r;
        let (d4, d6) = if a <= 0.0 { (d6, d7) } else { (r2, d16) };
        let d5 = d6 + 1.0;
        let u = u + d5;
        let u = (-u).mul_add(d4, d4);
        let u = d5 / u;
        d5 + u
    }

    /// The S-curve for positive shapes.
    fn s_curve(u: f64) -> f64 {
        let u = u + -0.5;
        let u = (-u).mul_add(u.abs(), u);
        u.mul_add(2.0, 0.5)
    }

    #[inline]
    fn step(&mut self) -> (f64, f64) {
        let x = (self.x - self.dec) * self.mul;
        let t = self.inv_sr + self.t;
        let x = if x < 0.0 { 0.0 } else { x };
        let x = if x > 1.0 { 1.0 } else { x };
        self.x = x;
        self.t = t;
        (x, t)
    }

    #[inline]
    fn past_end(x: f64) -> f64 {
        let y = x + -END;
        if y < 0.0 { 0.0 } else { y * END_SCALE }
    }

    /// One frame (`processSampleStage<stage>`).
    fn tick(&mut self) -> f32 {
        let (x, t) = match self.stage {
            1 => {
                let (x, t) = self.step();
                let mut u = 1.0 - x;
                let s = self.att_shape;
                if s != 0.0 {
                    u = Self::curve(u, s);
                }
                if s > 0.0 {
                    u = Self::s_curve(u);
                }
                self.level = u as f32;
                (x, t)
            }
            2 => {
                let (x, t) = self.step();
                let y = Self::past_end(x);
                let sus = self.sustain as f64;
                self.level = (1.0 - sus).mul_add(y, sus) as f32;
                (x, t)
            }
            4 => {
                let (x, t) = self.step();
                let y = Self::past_end(x);
                let s = self.rel_shape;
                let v = if s == -1.0 {
                    y
                } else if !self.shaped_release {
                    (y + -1.0) + 1.0
                } else {
                    let mut u = 1.0 - y;
                    if s != 0.0 {
                        u = Self::curve(u, s);
                    }
                    if s > 0.0 {
                        u = Self::s_curve(u);
                    }
                    1.0 - u
                };
                self.level = (v * self.rel_start as f64) as f32;
                (x, t)
            }
            _ => return self.level * (self.vel * self.gain),
        };
        if x <= END || t > self.t_end {
            self.advance();
        }
        self.level * (self.vel * self.gain)
    }
}

/// A one-multiply exponential decay (+0x710.., five of them): time, value, factor,
/// 1/rate, floor. The value falls to the floor and is then held at 0.
#[derive(Clone, Copy)]
struct Decay {
    time: f32,
    value: f32,
    coef: f32,
    inv_sr: f32,
    floor: f32,
}

impl Decay {
    fn new(floor: f32) -> Decay {
        Decay { time: 1.0, value: 0.0, coef: 0.5, inv_sr: 1.0, floor }
    }

    /// The factor `E(−ln2/(sr·t) · 86.5617 + 1200)`, clamped to 0..1.
    fn factor(k: f32, time: f32) -> f32 {
        clamp01(exp_e((k / (time + TINY)).mul_add(DECAY_K, 1200.0)))
    }

    fn set_time(&mut self, t: f32) {
        if self.time != t {
            self.time = t;
            self.coef = Self::factor(self.inv_sr * LN2_NEG, t);
        }
    }

    #[inline]
    fn tick(&mut self) -> f32 {
        let v = self.coef * self.value;
        self.value = v;
        if v >= self.floor {
            v - self.floor
        } else {
            self.value = 0.0;
            0.0
        }
    }
}

/// `DSP::DustyTomCore`.
pub struct Tronic {
    /// +0x80 the core parameters as last set (tune, decay, interval, impact, bend,
    /// FM freq, FM amount, FM decay).
    param: [f32; 8],
    /// +0xa0 Tune (semitones above MIDI 24), +0xa4 the played pitch, +0xa8 1/its Hz.
    tune_st: f32,
    pitch: f32,
    period: f32,
    /// +0xac Interval (semitones), +0xb0 Decay (dB), +0xb4 FM Freq (semitones), +0xb8 FM
    /// Amount.
    interval: f32,
    decay_db: f32,
    fm_st: f32,
    fm_amount: f32,
    /// +0xbc noise gain √(sr/44100).
    noise_gain: f32,
    /// +0xc0 the second oscillator's Hz.
    hz1: f32,
    /// +0xc4/+0xc8/+0xcc: the 2.2 ms wait before the click decays start.
    delay: f32,
    delay_inc: f32,
    delayed: bool,
    /// +0xd0 oscillator amplitudes, +0xe0 the velocities they scale.
    amp: [f32; 4],
    amp_vel: [f32; 4],
    /// +0xf0 oscillator phases, +0x100 Hz, +0x110 increments, +0x120 1/rate.
    phase: [f32; 4],
    hz: [f32; 4],
    inc: [f32; 4],
    inv_sr: f32,
    /// +0x150 FM phase, +0x154 Hz, +0x158 increment.
    fm_phase: f32,
    fm_hz: f32,
    fm_inc: f32,
    /// +0x164 noise scale, +0x16c the noise LCG.
    noise_scale: f32,
    seed: i32,
    /// The noise colouring: two 3 kHz low-passes and a 5 kHz high-pass.
    lp1: OnePole,
    lp2: OnePole,
    hp1: OnePole,
    /// +0x1f8 the noise band-pass at 650 Hz: band, low, g, damping, normaliser.
    svf_b: f32,
    svf_l: f32,
    svf_g: f32,
    svf_k: f32,
    svf_d: f32,
    /// +0x228 the DC blocker and its corner (+0x240).
    dc: OnePole,
    dc_hz: f32,
    /// +0x250 the decay-time clip, +0x260.. the envelope clip/saturation pairs.
    decay_clip: [f32; 2],
    env_clip: [[f32; 2]; 3],
    /// +0x290.. the soft clip per oscillator: knee, width and sign for the positive and
    /// the negative half-wave.
    knee: [[f32; 4]; 2],
    width: [[f32; 4]; 2],
    sign: [[f32; 4]; 2],
    /// +0x328/+0x32c the noise clip and saturation.
    noise_clip: [f32; 2],
    env: [FreeEnv; 4],
    /// +0x710 pitch drop, +0x724 body, +0x738 click, +0x74c click ramp, +0x760 FM.
    d_drop: Decay,
    d_body: Decay,
    d_click: Decay,
    d_ramp: Decay,
    d_fm: Decay,
    /// +0x774 voice: 0 idle, 1 playing, 2 fading; +0x780 fade, +0x784 its factor.
    state: i32,
    fade: f32,
    fade_coef: f32,
    /// +0x78c..+0x7b4: the cross-fade out of the voice a retrigger cuts.
    xf_hz: f32,
    xf_pos: f32,
    xf_ramp: f32,
    out_raw: f32,
    out_last: f32,
    out_prev: f32,
    xf_slope: f32,
    xf_from: f32,
    xf_scale: f32,
    xf_step: f32,
    xf_inv_sr: f32,
    /// +0x7d0 smoothed values, +0x810 increments, +0x850 targets (docs/synth-tom.md §3).
    cur: [f32; 16],
    step: [f32; 16],
    target: [f32; 16],
    /// +0x890 targets changed, +0x891 ramping, +0x894 ticks left, +0x898 ticks per
    /// ramp, +0x89c 1/frames per ramp, +0x8a0 ramp state.
    dirty: bool,
    ramping: bool,
    ticks: i32,
    ramp_ticks: i32,
    ramp_inv: f32,
    ramp_state: i32,
    /// +0x8b0 velocity, +0x8b4 note, +0x8b8 a voice ran.
    vel: f32,
    note: u32,
    ran: bool,
}

impl Tronic {
    pub fn new() -> Tronic {
        let t = c(0x3f2a_aaa8); // 0.6666665
        Tronic {
            param: [0.0; 8],
            tune_st: 0.0,
            pitch: 0.0,
            period: 0.0,
            interval: 0.0,
            decay_db: 0.0,
            fm_st: 0.0,
            fm_amount: 0.0,
            noise_gain: 0.0,
            hz1: 0.0,
            delay: 0.0,
            delay_inc: 0.0,
            delayed: false,
            amp: [0.0; 4],
            amp_vel: [0.0; 4],
            phase: [0.0; 4],
            hz: [1.0; 4],
            inc: [0.0; 4],
            inv_sr: 1.0,
            fm_phase: 0.0,
            fm_hz: 1.0,
            fm_inc: 0.0,
            noise_scale: c(0x3000_009d),
            seed: 0,
            lp1: OnePole::default(),
            lp2: OnePole::default(),
            hp1: OnePole::default(),
            svf_b: 0.0,
            svf_l: 0.0,
            svf_g: 0.0,
            svf_k: c(0x3e4c_ccd0),
            svf_d: 0.0,
            dc: OnePole::default(),
            dc_hz: 1.0,
            decay_clip: [c(0x3d75_c28f), c(0x4205_5556)],
            env_clip: [[2.0, 1.0], [2.0, 1.0], [1.0, 2.0]],
            knee: [[t; 4]; 2],
            width: [[t; 4]; 2],
            sign: [[1.0; 4], [-1.0; 4]],
            noise_clip: [16.0, 0.125],
            env: [FreeEnv::new(0.0, false), FreeEnv::new(0.0, false), FreeEnv::new(-0.5, false), FreeEnv::new(0.0, true)],
            d_drop: Decay::new(1e-4),
            d_body: Decay::new(1e-4),
            d_click: Decay::new(1e-4),
            d_ramp: Decay::new(c(0x3356_bf95)),
            d_fm: Decay::new(1e-4),
            state: 0,
            fade: 0.0,
            fade_coef: 0.0,
            xf_hz: 1.0,
            xf_pos: 1.0,
            xf_ramp: 0.0,
            out_raw: 0.0,
            out_last: 0.0,
            out_prev: 0.0,
            xf_slope: 0.0,
            xf_from: 0.0,
            xf_scale: 0.5,
            xf_step: 0.5,
            xf_inv_sr: 1.0,
            cur: [0.0; 16],
            step: [0.0; 16],
            target: [0.0; 16],
            dirty: true,
            ramping: false,
            ticks: 0,
            ramp_ticks: 0,
            ramp_inv: 0.0,
            ramp_state: 1,
            vel: 0.0,
            note: 0,
            ran: false,
        }
    }

    fn touch(&mut self) {
        self.dirty = true;
        self.ramp_state = 1;
    }

    /// +0x874/+0x878: the decay time (from Decay and the period) and its clipped
    /// share.
    fn decay_targets(&mut self, per3: f32) {
        let d = db_ms(self.decay_db + per3);
        self.target[9] = d;
        let [k, sat] = self.decay_clip;
        self.target[10] = soft(clamp_sym(d * c(0x3d8f_5c29), k), sat); // ·0.07
    }

    /// +0x880/+0x884: the FM pitch and the FM amount over its Hz.
    fn fm_targets(&mut self, f: f32) {
        self.target[12] = f;
        self.target[13] = self.fm_amount / pitch_hz(f * c(0x3f4c_cccd)); // ·0.8
    }

    /// `calcPitchParams`.
    fn calc_pitch(&mut self) {
        let p = self.tune_st + (self.note as f32 + -60.0);
        self.pitch = p;
        let hz = pitch_hz(p);
        self.period = 1.0 / hz;
        self.target[6] = p;
        self.target[5] = p + -40.0;
        let per = self.period * c(0x42a4_d53f); // ·82.4165
        self.target[7] = per * (c(0x4102_d6b6) / pitch_hz(self.interval)); // 8.17742/
        self.decay_targets(per.mul_add(3.0, -2.0));
        self.fm_targets(p + self.fm_st);
        self.touch();
    }

    /// `DustyTomCore::setParameter(idx, v)`.
    fn set_parameter(&mut self, idx: u32, v: f32) {
        if idx > 7 {
            return;
        }
        self.param[idx as usize] = v;
        match idx {
            0 => {
                self.tune_st = clamp01(v).mul_add(24.0, 24.0);
                self.calc_pitch();
                return;
            }
            1 => {
                self.decay_db = clamp01(v).mul_add(22.0, 34.0);
                let per = self.period * c(0x42a4_d53f);
                self.decay_targets(per.mul_add(3.0, -2.0));
            }
            2 => {
                let st = clamp01(v).mul_add(14.0, -0.5);
                self.interval = st;
                self.target[8] = st;
                let h = pitch_hz(st);
                self.target[7] = self.period * c(0x42a4_d53f) * (c(0x4102_d6b6) / h);
            }
            3 => {
                // Impact: v/0.3 and (v − 0.4)/0.6, clamped to 0..1.
                let cl = |x: f32| {
                    let x = if x < 0.0 { 0.0 } else { x };
                    if x > 1.0 { 1.0 } else { x }
                };
                self.target[0] = cl(v / c(0x3e99_999a));
                self.target[1] = cl((v + c(0xbecc_cccd)) / c(0x3f19_999a));
                self.target[2] = clamp01(v / c(0x3f66_6666));
                let k = clamp01(v / c(0x3e4c_cccd));
                self.target[3] = k.mul_add(c(0xbc23_d70a), c(0x3c23_d70a)) + c(0x3b83_126f);
                let d = clamp01((v + c(0xbe4c_cccd)) / c(0x3f4c_cccd));
                self.target[4] = d.mul_add(14.0, 1.0);
            }
            4 => self.target[11] = clamp01(v).mul_add(-20.0, 10.0),
            5 => {
                self.fm_st = clamp01(v).mul_add(70.0, 0.0);
                let f = self.fm_st + self.pitch;
                self.fm_targets(f);
            }
            6 => {
                self.fm_amount = clamp01(v).mul_add(60.0, 0.0);
                let f = self.pitch + self.fm_st;
                self.fm_targets(f);
            }
            _ => {
                let x = clamp01(self.param[7]);
                let ms = self.decay_db * c(0x3f99_999a) + -10.0; // ·1.2 − 10
                self.target[14] = db_ms(ms.mul_add(x, 10.0));
            }
        }
        self.touch();
    }

    /// `onReset`, and the end of a voice.
    fn clear_voice(&mut self) {
        self.vel = 0.0;
        self.ran = false;
        self.delay = 0.0;
        self.delayed = false;
        self.amp = [0.0; 4];
        self.amp_vel = [0.0; 4];
        self.cur = self.target;
        self.touch();
        self.phase = [0.0; 4];
        self.fm_phase = 0.0;
        self.lp1.s = 0.0;
        self.lp2.s = 0.0;
        self.hp1.s = 0.0;
        self.dc.s = 0.0;
        self.svf_b = 0.0;
        self.svf_l = 0.0;
        for e in self.env.iter_mut() {
            e.clear();
        }
        for d in [&mut self.d_drop, &mut self.d_body, &mut self.d_click, &mut self.d_ramp, &mut self.d_fm] {
            d.value = 0.0;
        }
        self.state = 0;
        self.fade = 0.0;
        self.xf_pos = 1.0;
        self.xf_ramp = 0.0;
        self.out_raw = 0.0;
        self.out_last = 0.0;
        self.out_prev = 0.0;
        self.xf_slope = 0.0;
        self.xf_from = 0.0;
    }

    fn active(&self) -> bool {
        (self.env[0].stage != 0 || self.env[1].stage != 0 || self.env[2].stage != 0) && self.state != 0
    }

    /// One frame of `processAudioImpl` (docs/synth-tom.md §3).
    #[inline]
    fn frame(&mut self) -> f32 {
        if self.ramping {
            for i in 0..16 {
                self.cur[i] += self.step[i];
            }
        }
        let cv = self.cur;
        // Noise: an LCG, two low-passes, a high-pass and a band-pass.
        self.seed = self.seed.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        let n = clamp_sym(self.noise_scale * self.seed as f32 * 10000.0, 1.0);
        let y1 = self.lp1.lowpass(self.noise_gain * n);
        let y2 = self.lp2.lowpass(y1);
        let x = self.hp1.highpass(y2);
        let (g, k, b, l) = (self.svf_g, self.svf_k, self.svf_b, self.svf_l);
        let b2 = b + b;
        let t = l.mul_add(2.0, g * b2);
        let gg = g.mul_add(k, g * g);
        let t = b2.mul_add(k, t);
        let t = x.mul_add(gg, t);
        let noise_hp = x - self.svf_d * t;
        let bp = g.mul_add(noise_hp, b);
        let lp = g.mul_add(b + bp, l);
        self.svf_b = bp;
        self.svf_l = lp;
        // Decays.
        self.d_body.set_time(cv[10]);
        if !self.delayed {
            if self.delay >= 1.0 {
                self.d_click.value = 1.0;
                self.d_body.value = 1.0;
                self.delayed = true;
            } else {
                self.delay += self.delay_inc;
            }
        }
        let click = self.d_click.tick();
        let body = self.d_body.tick();
        let dec = cv[9];
        self.env[0].set_decay(dec);
        self.env[1].set_decay(dec);
        self.env[3].set_decay(dec + dec);
        let per = cv[7];
        self.env[2].set_decay(per * c(0x3ca3_d70a)); // ·0.02
        self.d_drop.set_time(per * c(0x3cf5_c28f)); // ·0.03
        let r = per * c(0x3a37_8034); // ·0.0007
        let rmax = c(0x3abe_0ded); // 0.00145
        self.d_ramp.set_time(if r > rmax { rmax } else { r });
        // Oscillator amplitudes.
        self.env[2].gain = cv[0];
        let [[k0, s0], [k1, s1], [k2, s2]] = self.env_clip;
        let e0 = soft(clamp_sym(self.env[0].tick(), k0), s0);
        let e1 = soft(clamp_sym(self.env[1].tick(), k1), s1);
        let e2 = soft(clamp_sym(self.env[2].tick(), k2), s2);
        let lanes = [e0, e1, e2, 0.0];
        for (i, l) in lanes.iter().enumerate() {
            self.amp[i] = self.amp_vel[i] * l;
        }
        // Pitch and FM.
        let (pst, ist, bend) = (cv[6], cv[8], cv[11]);
        let e3 = self.env[3].tick();
        let sum = pst + ist;
        let base = pitch_hz(sum);
        let bent_st = e3.mul_add(bend, sum - bend);
        let bent = pitch_hz(bent_st);
        let drop = self.d_drop.tick();
        self.fm_hz = pitch_hz((bent_st - sum) + cv[12]);
        self.fm_inc = clamp_sym(self.fm_hz * self.inv_sr, 1.0);
        self.d_fm.set_time(cv[14]);
        let fm_env = self.d_fm.tick();
        let fp = self.fm_inc + self.fm_phase + 10.0;
        let ri = (fp + 0.5) as i32 as f32;
        let frac = fp - ri;
        self.fm_phase = frac;
        let off = bent - base;
        let drop = drop.mul_add(c(0x3ea8_f5c3), 1.0); // 1 + 0.33·drop
        let fm = frac * (fp - ri).abs().mul_add(-16.0, 8.0) * fm_env * cv[13];
        // Oscillators: 2^(st/12 + 3.0313597) Hz, phase, FM'd triangle.
        let st = [pst, sum, pst + c(0x418c_147b), 0.0];
        let mut tri = [0.0f32; 4];
        for i in 0..4 {
            let hz = (off + exp2_poly(st[i] * c(0x3daa_aaab) + c(0x4042_01cc))) * drop;
            self.hz[i] = hz;
            let inc = hz * self.inv_sr;
            let inc = if inc < 1.0 { inc } else { 1.0 };
            let inc = if inc > -1.0 { inc } else { -1.0 };
            self.inc[i] = inc;
            let p = inc + self.phase[i] + 10.0;
            let ph = p - trunc24(p + 0.5);
            self.phase[i] = ph;
            let q = fm + ph + 10.0;
            let f = q - trunc24(q + 0.5);
            tri[i] = (f + f).abs() + -0.5;
        }
        self.hz1 = self.hz[1];
        // Gains and the soft clip.
        let ramp = self.d_ramp.tick();
        let noise = noise_hp * 80.0;
        let rr = ramp * 2000.0;
        let rr = if rr > 1.0 { 1.0 } else { rr };
        let rr = if rr < 0.0 { 0.0 } else { rr };
        let m1 = (cv[4] + -1.0).mul_add(rr, 1.0) * 4.0;
        let gains = [4.0, m1, 4.0, 0.0];
        let rq = recip(c(0x3f49_0fdb));
        let mut out = [0.0f32; 4];
        for i in 0..4 {
            let v = tri[i] * gains[i];
            let h = usize::from(v.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater));
            let a = v.abs();
            let (knee, w, sign) = (self.knee[h][i], self.width[h][i], self.sign[h][i]);
            let d = (a - knee).min(w + w) * c(0x3f49_0fdb); // ·π/4
            let x = d * recip(w);
            let x2 = x * x;
            let p = x2 * c(0xb2cd_4ca8) + c(0x3638_b94f);
            let p = x2 * p + c(0xb950_0c03);
            let p = x2 * p + c(0x3c08_8885);
            let p = x2 * p + c(0xbe2a_aac1);
            let p = x2 * p;
            let s = x * (p + 1.0) * w;
            let s = rq * s + knee;
            let s = if knee > a { a } else { s };
            out[i] = s * sign * [1.5, 1.5, 2.0, 0.0][i];
        }
        out[1] += body * (noise * c(0x3b65_6042) * cv[2]); // noise ·0.0035
        let mut lane = [0.0f32; 4];
        for i in 0..4 {
            let shaped = out[i] * (out[i].abs() * c(0x3f59_999a) + c(0x3e19_9998));
            lane[i] = self.amp[i] * shaped;
        }
        let mut y = (lane[0] + lane[1]) + (lane[2] + lane[3]);
        // The cross-fade from a cut voice.
        self.out_prev = self.out_last;
        self.out_raw = y;
        if self.xf_pos > 0.0 {
            let p = self.xf_pos - self.xf_step;
            self.xf_pos = p;
            let h = p + -0.5;
            let h = (-h).mul_add(h.abs(), h).mul_add(2.0, 0.5);
            let r = self.xf_from + self.xf_ramp;
            self.xf_ramp += self.xf_slope;
            y += r * h;
        }
        self.out_last = y;
        // DC blocker at 6 × the pitch 40 semitones down.
        let f = pitch_hz(cv[5]) * 6.0;
        if self.dc_hz != f {
            self.dc_hz = f;
            self.dc.tune(f * PI * self.inv_sr);
        }
        let hp = self.dc.highpass(y);
        // Noise, velocity and the voice fade.
        let nz = soft(clamp_sym(noise, self.noise_clip[0]), self.noise_clip[1]);
        let vel = self.vel;
        let velc = clamp01(vel);
        let fade = match self.state {
            0 => 0.0,
            1 => 1.0,
            _ => {
                let f = self.fade_coef * self.fade;
                self.fade = f;
                if (f as f64) < END {
                    self.fade = 0.0;
                    self.state = 0;
                    0.0
                } else {
                    f
                }
            }
        };
        let ns = velc.mul_add(0.5, 0.5) * (click * nz);
        let ns = vel * cv[1] * ns;
        ((y + hp) + ns) * c(0x3d1b_8994) * fade // ·0.037973
    }
}

impl Engine for Tronic {
    fn set_sample_rate(&mut self, sr: f32) {
        let steps = ((sr * c(0x3ca3_d70a)) / 128.0 + 0.5) as i32; // 20 ms
        let steps = if steps > 1 { steps } else { 1 };
        self.ramp_ticks = steps;
        self.ramp_inv = 1.0 / (steps * 128) as f32;
        let inv = 1.0 / sr;
        self.inv_sr = inv;
        for i in 0..4 {
            let v = self.hz[i] * inv;
            let v = if v < 1.0 { v } else { 1.0 };
            self.inc[i] = if v > -1.0 { v } else { -1.0 };
        }
        self.fm_inc = clamp_sym(inv * self.fm_hz, 1.0);
        self.lp1.tune(inv * c(0x4613_431d)); // π·3000
        self.lp1.s = 0.0;
        self.lp2.tune(inv * c(0x4613_431d));
        self.lp2.s = 0.0;
        self.hp1.tune(inv * c(0x4675_6fdb)); // π·5000
        self.hp1.s = 0.0;
        let f = (sr * c(0x3efa_e148)).min(650.0); // fminnm(0.49·sr, 650)
        self.svf_g = tan_approx(inv * (f * PI));
        self.svf_d = 1.0 / self.svf_g.mul_add(self.svf_g + self.svf_k, 1.0);
        self.svf_b = 0.0;
        self.svf_l = 0.0;
        if self.dc_hz != -1.0 {
            self.dc.tune(inv * (self.dc_hz * PI));
        }
        self.dc_hz = -1.0;
        self.dc.s = 0.0;
        self.env[0].set_sample_rate(sr);
        self.env[0].set_attack(c(0x3ca3_d70a)); // 20 ms
        self.env[0].set_release_zero();
        self.env[1].set_sample_rate(sr);
        self.env[1].set_release_zero();
        self.env[2].set_sample_rate(sr);
        self.env[2].set_attack(c(0x3a03_126f)); // 0.5 ms
        self.env[2].set_release_zero();
        self.env[3].set_sample_rate(sr);
        self.env[3].set_attack(0.0);
        self.env[3].set_release_zero();
        let k = inv * LN2_NEG;
        for d in [&mut self.d_drop, &mut self.d_body, &mut self.d_click, &mut self.d_ramp, &mut self.d_fm] {
            d.inv_sr = inv;
            d.value = 0.0;
            d.coef = Decay::factor(k, d.time);
        }
        self.d_click.set_time(c(0x3a03_126f)); // 0.5 ms
        self.state = 0;
        self.fade = 0.0;
        let fc = (k / c(0x3ba3_d70a)).exp(); // 5 ms
        self.fade_coef = if fc > 1.0 { 1.0 } else { fc };
        self.xf_inv_sr = inv;
        let s = inv * self.xf_hz;
        self.xf_pos = 1.0;
        self.xf_ramp = 0.0;
        self.out_raw = 0.0;
        self.out_last = 0.0;
        self.out_prev = 0.0;
        self.xf_slope = 0.0;
        self.xf_from = 0.0;
        self.xf_scale = 2.0;
        self.xf_step = s + s;
        self.noise_gain = (sr / 44100.0).sqrt();
        self.delay_inc = 1.0 / (sr * c(0x3b10_2de0)); // 2.2 ms
    }

    fn reset(&mut self) {
        self.clear_voice();
    }

    fn set_shared(&mut self, idx: u32, v: f32, _shared: &[f32; 7]) {
        match idx {
            1 => self.set_parameter(0, v),
            2 => self.set_parameter(1, v),
            3 => self.set_parameter(4, v),
            4 => self.set_parameter(3, v),
            _ => {}
        }
    }

    fn update_global(&mut self, s: &[f32; 7]) {
        self.set_parameter(0, s[1]);
        self.set_parameter(1, s[2]);
        self.set_parameter(4, s[3]);
        self.set_parameter(3, s[4]);
    }

    fn set_core(&mut self, idx: u32, v: f32) {
        self.set_parameter(idx, v);
    }

    fn note_on(&mut self, vel: f32, note: u32, shared: &[f32; 7]) {
        // `TomProcessor::setNoteOn`: Velocity bends the response, then cubes it.
        let vs = shared[6];
        let x = if vs < 0.0 { vel } else { 1.0 - vel };
        let v = (-x).mul_add(vs.abs(), 1.0);
        let v = v * (v * v);
        if !(0.0..=1.0).contains(&v) {
            self.vel = 0.0;
            return;
        }
        self.note = note;
        self.calc_pitch();
        if self.active() {
            // A retrigger: the values jump to their targets (the DC corner keeps
            // gliding).
            self.cur[0..5].copy_from_slice(&self.target[0..5]);
            self.cur[6..15].copy_from_slice(&self.target[6..15]);
        } else {
            self.cur = self.target;
            self.touch();
        }
        let click = self.cur[3];
        self.phase = [c(0x3f4f_1aa0), c(0x3f4f_1aa0), c(0x3f0c_cccd), 0.25];
        self.env[1].set_attack(click);
        for e in self.env.iter_mut() {
            e.note_on(v);
        }
        self.d_drop.value = 1.0;
        for i in 0..4 {
            let m = self.amp[i];
            self.amp_vel[i] = if v.is_nan() || m.is_nan() { f32::NAN } else { v.max(m) };
        }
        self.d_ramp.value = 1.0;
        self.d_fm.value = 1.0;
        self.state = 1;
        self.fade = 0.0;
        self.xf_hz = self.hz1;
        self.xf_step = self.hz1 * self.xf_inv_sr * self.xf_scale;
        self.xf_pos = 1.0;
        self.xf_ramp = 0.0;
        self.xf_slope = self.out_last - self.out_prev;
        self.xf_from = self.out_raw;
        self.delay = 0.0;
        self.delayed = false;
        self.vel = v;
        self.ran = true;
    }

    fn note_off(&mut self, _note: u32) {}

    fn choke(&mut self) {
        if self.state == 1 {
            self.state = 2;
            self.fade = 1.0;
        }
    }

    fn control_rate(&mut self) {
        if !self.dirty {
            self.ticks = self.ticks.wrapping_sub(1);
            if self.ticks == 0 {
                self.cur = self.target;
                self.step = [0.0; 16];
                self.ramping = false;
                self.ramp_state = 4;
            } else if self.ramp_state == 2 {
                self.ramp_state = 3;
            } else if self.ramp_state == 4 {
                self.ramp_state = 0;
            }
            return;
        }
        self.dirty = false;
        self.ramp_state = 2;
        let mut any = false;
        for i in 0..16 {
            let d = (self.target[i] - self.cur[i]) * self.ramp_inv;
            if d * d < 1e-15 {
                self.step[i] = 0.0;
                self.cur[i] = self.target[i];
            } else {
                self.step[i] = d;
                any = true;
            }
        }
        if !any {
            self.ramp_state = 4;
            return;
        }
        self.ticks = self.ramp_ticks;
        self.ramping = true;
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        if !self.active() {
            if self.ran {
                self.clear_voice();
            }
            return;
        }
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            let y = self.frame();
            *a = y;
            *b = y;
        }
    }

    fn silent(&self) -> bool {
        !self.active()
    }
}
