//! Grit (`NI::DSP::GritKickCore`, KickProcessor +0x1c80; docs/synth-kick.md §4.4): a
//! triangle-folded sine body with a pitch envelope, a noise burst through a 4-lane SVF
//! bank, a peak-held click, two modulated combs with sample-and-hold "aero" grit, and an
//! EQ chain. With Gate on, three envelopes hold at their sustain while the note is held
//! and three hold-gates fade on note-off. Offsets in comments are the C++ members.

use super::env::{Attack, Decay, END, FreeEnv, Release};
use super::simd::recip;
use super::smooth::Smooth;
use super::sub::{exp2_poly, tan_approx};
use crate::fx::exp_e;
use std::f32::consts::PI;

/// `-ln 2` (`0xbf317215`).
const NEG_LN2: f32 = f32::from_bits(0xbf31_7215);
/// `1381.8816`: `E(5·p + this)` is the frequency of MIDI pitch `p`.
const PITCH: f32 = f32::from_bits(0x44ac_bc36);
/// The delay buffers' length.
const LEN: usize = 0x10000;
const MASK: u32 = 0xffff;

#[inline]
fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    let x = if x < lo { lo } else { x };
    if x > hi { hi } else { x }
}

/// A decaying one-shot (`v *= coef`, output `v − thr`): the pitch drop and click levels
/// (+0x508, +0x51c, +0x530, +0x544).
#[derive(Clone, Copy, Debug)]
struct Pd {
    time: f32,
    v: f32,
    coef: f32,
    inv: f32,
    thr: f32,
}

impl Pd {
    fn new() -> Pd {
        Pd { time: 1.0, v: 0.0, coef: 0.0, inv: 1.0, thr: 1e-4 }
    }
    fn coef_for(inv: f32, time: f32) -> f32 {
        let x = (inv * NEG_LN2) / (time + f32::from_bits(0x0da2_4260));
        let e = exp_e(x.mul_add(f32::from_bits(0x42ad_1f97), 1200.0));
        clamp(e, 0.0, 1.0)
    }
    fn set_sr(&mut self, inv: f32) {
        self.inv = inv;
        self.v = 0.0;
        self.coef = Self::coef_for(inv, self.time);
    }
    fn set_time(&mut self, t: f32) {
        if self.time != t {
            self.time = t;
            self.coef = Self::coef_for(self.inv, t);
        }
    }
    #[inline]
    fn tick(&mut self) -> f32 {
        let v = self.coef * self.v;
        self.v = v;
        if v >= self.thr {
            v - self.thr
        } else {
            self.v = 0.0;
            0.0
        }
    }
}

/// A gate that fades when released (0 off, 1 open, 2 fading): the hold-gates (+0x558,
/// +0x570, +0x588) and the output gate (+0x140934).
#[derive(Clone, Copy, Debug, Default)]
struct Gate {
    state: u32,
    v: f32,
    coef: f32,
}

impl Gate {
    #[inline]
    fn tick(&mut self) -> f32 {
        match self.state {
            0 => 0.0,
            1 => 1.0,
            _ => {
                let v = self.coef * self.v;
                self.v = v;
                if (v as f64) < END {
                    self.v = 0.0;
                    self.state = 0;
                    0.0
                } else {
                    v
                }
            }
        }
    }
    fn release(&mut self) {
        if self.state == 1 {
            self.state = 2;
            self.v = 1.0;
        }
    }
}

/// A one-pole (TPT) filter.
#[derive(Clone, Copy, Debug)]
struct OnePole {
    z: f32,
    a: f32,
    b: f32,
    g: f32,
    hz: f32,
    inv: f32,
}

impl OnePole {
    fn new(hz: f32) -> OnePole {
        OnePole { z: 0.0, a: 0.0, b: 0.0, g: 0.0, hz, inv: 1.0 }
    }
    fn set_hz(&mut self, hz: f32) {
        self.hz = hz;
        let g = tan_approx((hz * PI) * self.inv);
        self.g = g;
        self.a = 1.0 / (g + 1.0);
        self.b = g + -1.0;
    }
    /// `if hz changed: set_hz`.
    fn update(&mut self, hz: f32) {
        if self.hz != hz {
            self.set_hz(hz);
        }
    }
    #[inline]
    fn lp(&mut self, x: f32) -> f32 {
        let v = self.a * (-self.b).mul_add(self.z, x);
        let y = self.g * (self.z + v);
        self.z = v;
        y
    }
    #[inline]
    fn hp(&mut self, x: f32) -> f32 {
        let v = self.a * (-self.b).mul_add(self.z, x);
        let y = v - self.z;
        self.z = v;
        y
    }
}

/// A peaking / band biquad (`a0·(g·(x − x2) − a1·y1 − a2·y2)`), with `kk` the peak gain
/// for the caller's `x + kk·y`.
#[derive(Clone, Copy, Debug)]
struct Biquad {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
    fc: f32,
    bw: f32,
    gain: f32,
    a0: f32,
    a1: f32,
    a2: f32,
    g: f32,
    g2: f32,
    gk: f32,
    k: f32,
    kk: f32,
    inv: f32,
}

impl Biquad {
    fn new() -> Biquad {
        Biquad {
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
            fc: 1.0,
            bw: 0.0,
            gain: 0.0,
            a0: 0.0,
            a1: 0.0,
            a2: 0.0,
            g: 0.0,
            g2: 0.0,
            gk: 0.0,
            k: 0.0,
            kk: 0.0,
            inv: 1.0,
        }
    }
    /// A new centre frequency with the current `k`.
    fn set_fc(&mut self, fc: f32) {
        self.fc = fc;
        let g = tan_approx((fc * PI) * self.inv);
        self.g = g;
        let g2 = g * g;
        self.g2 = g2;
        let gk = self.k * g;
        self.gk = gk;
        let p = g2 + 1.0;
        self.a0 = 1.0 / (gk + p);
        let m = g2 + -1.0;
        self.a1 = m + m;
        self.a2 = p - gk;
    }
    fn update_fc(&mut self, fc: f32) {
        if self.fc != fc {
            self.set_fc(fc);
        }
    }
    /// A new gain (dB) with the current bandwidth (octaves) and `g`.
    fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
        let a = exp2_poly(gain * f32::from_bits(0x3daa_aa7e));
        let bw = if self.bw < 0.1 { 0.1 } else { self.bw };
        let h = if bw > 10.0 { 5.0 } else { bw * 0.5 };
        let b = exp2_poly(h);
        let k = b.mul_add(b, -1.0) / (a * b);
        let k = clamp(k, 0.01, 100.0);
        self.k = k;
        self.kk = a.mul_add(a, -1.0) * k;
        let gk = self.g * k;
        self.gk = gk;
        let p = self.g2 + 1.0;
        self.a0 = 1.0 / (p + gk);
        let m = self.g2 + -1.0;
        self.a1 = m + m;
        self.a2 = p - gk;
    }
    fn update_gain(&mut self, gain: f32) {
        if self.gain != gain {
            self.set_gain(gain);
        }
    }
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let s = self.g * (x - self.x2);
        self.x2 = self.x1;
        self.x1 = x;
        let s = (-self.a1).mul_add(self.y1, s);
        let s = (-self.a2).mul_add(self.y2, s);
        let y = self.a0 * s;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
    fn clear(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

/// A state-variable filter (fused); `process` returns `z2 + lp`.
#[derive(Clone, Copy, Debug)]
struct Svf {
    z1: f32,
    z2: f32,
    g: f32,
    k: f32,
    hz: f32,
    d: f32,
    sr: f32,
    inv: f32,
}

impl Svf {
    fn set_hz(&mut self, hz: f32) {
        let f = clamp(hz, 0.0, self.sr * f32::from_bits(0x3efa_e148));
        self.hz = f;
        let g = tan_approx(self.inv * (f * PI));
        self.g = g;
        self.d = 1.0 / g.mul_add(self.k + g, 1.0);
    }
    fn update(&mut self, hz: f32) {
        if self.hz != hz {
            self.set_hz(hz);
        }
    }
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let (g, k, z1, z2) = (self.g, self.k, self.z1, self.z2);
        let a = z1 + z1;
        let b = z2.mul_add(2.0, g * a);
        let gg = g.mul_add(k, g * g);
        let c = a.mul_add(k, b);
        let h = x - self.d * x.mul_add(gg, c);
        let bp = g.mul_add(h, z1);
        let s = z1 + bp;
        let lp = g.mul_add(s, z2);
        self.z1 = bp;
        self.z2 = lp;
        z2 + lp
    }
}

/// The 4-lane NEON SVF bank (+0x690), unfused, `1/x` by FRECPE/FRECPS.
#[derive(Clone, Debug)]
struct Bank {
    z1: [f32; 4],
    z2: [f32; 4],
    g: [f32; 4],
    k: [f32; 4],
    hz: [f32; 4],
    d: [f32; 4],
    sr: f32,
    inv: f32,
}

impl Bank {
    fn set(&mut self, hz: [f32; 4]) {
        let lim = self.sr * f32::from_bits(0x3efa_e148);
        for (i, &f) in hz.iter().enumerate() {
            let h = if f < lim { f } else { lim };
            let h = if h > 0.0 { h } else { 0.0 };
            self.hz[i] = h;
            let w = (h * PI) * self.inv;
            let w2 = w * w;
            let w3 = w * w2;
            let w5 = w2 * w3;
            let g = w + (w3 * f32::from_bits(0x3eaa_aaab) + w5 * f32::from_bits(0x3e08_8886));
            self.g[i] = g;
            self.d[i] = recip(g * (self.k[i] + g) + 1.0);
        }
    }
    /// One sample per lane: `z1 + bp`.
    #[inline]
    fn process(&mut self, x: [f32; 4]) -> [f32; 4] {
        let mut out = [0f32; 4];
        for i in 0..4 {
            let (g, k, z1, z2) = (self.g[i], self.k[i], self.z1[i], self.z2[i]);
            let a = z1 + z1;
            let b = (z2 + z2) + g * a;
            let gg = g * g + g * k;
            let c = b + a * k;
            let h = x[i] - self.d[i] * (c + x[i] * gg);
            let v1 = z1 + g * h;
            let p = z1 + v1;
            let v2 = z2 + g * p;
            out[i] = p;
            self.z1[i] = v1;
            self.z2[i] = v2;
        }
        out
    }
}

/// `Delay1p`: a 64K-sample delay read `n` samples behind the write.
#[derive(Clone, Debug)]
struct Delay1 {
    buf: Vec<f32>,
    w: u32,
    r: u32,
    sr: f32,
}

impl Delay1 {
    fn new() -> Delay1 {
        Delay1 { buf: vec![0.0; LEN], w: 0, r: 0, sr: 1.0 }
    }
    fn set_delay(&mut self, t: f32) {
        let s = self.sr * t;
        let h = if s < 0.0 { -0.5f64 } else { 0.5 };
        let n = (h + s as f64) as i32 as u32;
        if n < 0x10000 {
            self.r = self.w.wrapping_sub(n) & MASK;
        }
    }
    #[inline]
    fn push(&mut self, x: f32) -> f32 {
        self.buf[self.w as usize] = x;
        let y = self.buf[self.r as usize];
        self.w = (self.w + 1) & MASK;
        self.r = (self.r + 1) & MASK;
        y
    }
}

/// `Delay2p`: a 64K-sample delay with a linear fractional read.
#[derive(Clone, Debug)]
struct Delay2 {
    buf: Vec<f32>,
    w: u32,
    r1: u32,
    r2: u32,
    frac: f32,
    sr: f32,
}

impl Delay2 {
    fn new() -> Delay2 {
        Delay2 { buf: vec![0.0; LEN], w: 0, r1: 0, r2: 0, frac: 0.0, sr: 1.0 }
    }
    fn set_delay(&mut self, t: f32) {
        let d = self.sr * t;
        let x = d + -0.5;
        let h = if x < 0.0 { -0.5f64 } else { 0.5 };
        let n = (h + x as f64) as i32;
        let n = if n < 0 { 0 } else { n } as u32;
        if n >= 0x10000 {
            return;
        }
        self.r1 = self.w.wrapping_sub(n) & MASK;
        self.r2 = self.w.wrapping_add(!n) & MASK;
        self.frac = d - n as f32;
    }
    /// The delayed sample; then `write` the input.
    #[inline]
    fn read(&mut self) -> f32 {
        let a = self.buf[self.r2 as usize];
        self.r2 = (self.r2 + 1) & MASK;
        let b = self.buf[self.r1 as usize];
        (a - b).mul_add(self.frac, b)
    }
    #[inline]
    fn write(&mut self, x: f32) {
        self.buf[self.w as usize] = x;
        self.w = (self.w + 1) & MASK;
        self.r1 = (self.r1 + 1) & MASK;
    }
}

/// A sample-and-hold clocked at `rate` (+0x1408fc, +0x140918).
#[derive(Clone, Copy, Debug, Default)]
struct Hold {
    phase: f32,
    inc: f32,
    held: f32,
    rate: f32,
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
        self.phase = clamp(p, -0.5, 0.5);
        h
    }
}

/// Smoothed values (`+0x140960 + 4·i`).
const DECAY_T: usize = 0;
const PD_T: usize = 1;
const PD_AMT: usize = 2;
const PD_LIM: usize = 3;
const CLICK: usize = 4;
const B_REL: usize = 5;
const PERIOD: usize = 6;
const PITCH_ST: usize = 7;
const ST: usize = 8;
const HZ: usize = 9;
const IMPACT: usize = 10;
const IMPACT_C: usize = 11;
const SH_ST: usize = 12;
const SVF_HZ: usize = 13;
const COMB_T: usize = 14;
const GRIND4: usize = 15;
const GRIND_MIX: usize = 16;
const LP_MOD: usize = 17;
const EQ_GAIN: usize = 18;
const GRIND_DB: usize = 19;
const AERO_HZ: usize = 20;
const AERO: usize = 21;

pub struct Grit {
    /// +0x80: the raw parameters (0 tune, 1 decay, 2 bend, 3 impact, 4 gate, 5 grind,
    /// 6 aero amount).
    raw: [f32; 7],
    /// +0x9c phase, +0xa8 1/sr.
    phase: f32,
    inv: f32,
    /// +0xb8 / +0xf0 noise scales, +0xc0 / +0xf8 the generators.
    noise1: f32,
    seed1: i32,
    noise2: f32,
    seed2: i32,
    env_a: FreeEnv,
    env_b: FreeEnv,
    env_c: FreeEnv,
    env_d: FreeEnv,
    pd: [Pd; 4],
    gates: [Gate; 3],
    /// +0x5a8/+0x5ac body clip and shape, +0x5b8/+0x5bc click clip and shape,
    /// +0x5c8/+0x5cc the env-C decay clip and shape.
    clip_a: f32,
    k_a: f32,
    clip_b: f32,
    k_b: f32,
    clip_c: f32,
    k_c: f32,
    bq1: Biquad,
    bq2: Biquad,
    op1: OnePole,
    op2: OnePole,
    bank: Bank,
    svf1: Svf,
    op3: OnePole,
    bq3: Biquad,
    op4: OnePole,
    op5: OnePole,
    bq4: Biquad,
    svf2: Svf,
    bq5: Biquad,
    dl: [Delay1; 3],
    comb1: Delay2,
    comb2: Delay2,
    sh1: Hold,
    sh2: Hold,
    out_gate: Gate,
    sm: Smooth<24>,
    /// +0x140aa0 note − 60, +0x140aa4 pitch (st), +0x140aa8 its frequency.
    note: f32,
    st: f32,
    hz: f32,
    /// +0x140aac Gate, +0x140ab0 velocity, +0x140ab4 played.
    gate: bool,
    vel: f32,
    played: bool,
    /// +0x140ac0 the bank feedback; levels (+0x140ac8 A, +0x140ad0 C, +0x140ad8 B,
    /// +0x140ae0 E) and the last outputs they hold at a retrigger.
    fb: f32,
    a_out: f32,
    lvl_a: f32,
    c_out: f32,
    lvl_c: f32,
    b_out: f32,
    lvl_b: f32,
    e_out: f32,
    lvl_e: f32,
    /// The `processAudioImpl<…, true>` (recomputing) variants are in use.
    first: bool,
}

impl Grit {
    /// `GritKickCore()` then `onSetSampleRate(sr, 128)`.
    pub fn new(sr: f32) -> Grit {
        let mut env_a = FreeEnv::new(Decay::Shape, Release::Lin);
        env_a.attack_kind = Attack::Inverted;
        env_a.gated = true;
        env_a.sm_hz = 1.0;
        let mut env_b = FreeEnv::new(Decay::Lin, Release::Log);
        env_b.sm_hz = 1.0;
        let mut env_c = FreeEnv::new(Decay::Log, Release::Lin);
        env_c.gated = true;
        env_c.sm_hz = 1.0;
        let mut env_d = FreeEnv::new(Decay::Log, Release::Lin);
        env_d.smooth = true;
        env_d.sm_hz = 1.0;
        let svf = |k: f32| Svf { z1: 0.0, z2: 0.0, g: 0.0, k, hz: 1.0, d: 1.0, sr: 1.0, inv: 1.0 };
        let mut g = Grit {
            raw: [0.0; 7],
            phase: 0.0,
            inv: 1.0,
            noise1: 0.0,
            seed1: 0,
            noise2: 0.0,
            seed2: 0,
            env_a,
            env_b,
            env_c,
            env_d,
            pd: [Pd::new(); 4],
            gates: [Gate::default(); 3],
            clip_a: f32::from_bits(0x3f87_ae14),
            k_a: f32::from_bits(0x3ff1_826b),
            clip_b: 2.0,
            k_b: 1.0,
            clip_c: f32::from_bits(0x3e99_999a),
            k_c: f32::from_bits(0x40d5_5555),
            bq1: Biquad::new(),
            bq2: Biquad::new(),
            op1: OnePole::new(1.0),
            op2: OnePole::new(1.0),
            bank: Bank {
                z1: [0.0; 4],
                z2: [0.0; 4],
                g: [0.0; 4],
                k: [f32::from_bits(0x3f99_999a), f32::from_bits(0x3fe6_6666), f32::from_bits(0x3fcc_cccd), f32::from_bits(0x3fcc_cccd)],
                hz: [1.0; 4],
                d: [0.0; 4],
                sr: 1.0,
                inv: 1.0,
            },
            svf1: svf(2.0),
            op3: OnePole::new(1.0),
            bq3: Biquad::new(),
            op4: OnePole::new(1.0),
            op5: OnePole::new(1.0),
            bq4: Biquad::new(),
            svf2: svf(f32::from_bits(0x3fcc_cccd)),
            bq5: Biquad::new(),
            dl: [Delay1::new(), Delay1::new(), Delay1::new()],
            comb1: Delay2::new(),
            comb2: Delay2::new(),
            sh1: Hold::default(),
            sh2: Hold::default(),
            out_gate: Gate::default(),
            sm: Smooth::new(),
            note: 0.0,
            st: 0.0,
            hz: 1.0,
            gate: false,
            vel: 0.0,
            played: false,
            fb: 0.0,
            a_out: 0.0,
            lvl_a: 0.0,
            c_out: 0.0,
            lvl_c: 0.0,
            b_out: 0.0,
            lvl_b: 0.0,
            e_out: 0.0,
            lvl_e: 0.0,
            first: true,
        };
        g.set_sample_rate(sr, 128);
        g
    }

    /// `onSetSampleRate(sr, block)` (0x1018eeff8).
    fn set_sample_rate(&mut self, sr: f32, block: i32) {
        self.sm.set_rate(sr, block);
        let a = &mut self.env_a;
        a.set_samplerate(sr);
        a.set_attack(f32::from_bits(0x39d1_b717));
        a.set_decay(1.0);
        a.set_release(f32::from_bits(0x3ba3_d70a));
        a.sustain = 0.0;
        a.att_shape = -0.5;
        a.dec_shape = -f64::from(f32::from_bits(0x3e4c_cccd));
        a.dec_coef = (super::env::LN_EIGHTH / (a.dec_time * a.sr as f64)).exp();
        a.gain = 1.0;
        a.vel_sens = 0.0;
        a.hold = self.gate;
        let b = &mut self.env_b;
        b.set_samplerate(sr);
        b.set_attack(f32::from_bits(0x3c23_d70a));
        b.set_decay(f32::from_bits(0x3d23_d70a));
        b.set_release(1.0);
        b.sustain = f32::from_bits(0x3f33_3333);
        b.dec_shape = 0.0;
        b.att_shape = 0.0;
        b.dec_coef = (super::env::LN_EIGHTH / (b.dec_time * b.sr as f64)).exp();
        b.rel_shape = -1.0;
        b.rel_coef = (super::env::LN_EIGHTH / (b.rel_raw * b.sr as f64)).exp();
        b.gain = 1.0;
        b.vel_sens = 0.0;
        let c = &mut self.env_c;
        c.set_samplerate(sr);
        c.set_attack(f32::from_bits(0x3ba3_d70a));
        c.set_decay(1.0);
        c.set_release(f32::from_bits(0x3ba3_d70a));
        c.sustain = 0.0;
        c.att_shape = 0.0;
        c.rel_shape = 0.0;
        c.rel_coef = 0.0;
        c.rel_time = (c.rel_raw + c.rel_raw) + f64::from_bits(0x3bc7_9ca1_0000_0000);
        c.inv_rel = 1.0 / c.rel_time;
        c.hold = self.gate;
        c.gain = 1.0;
        c.vel_sens = 0.0;
        let d = &mut self.env_d;
        d.set_samplerate(sr);
        d.set_attack(1.0);
        d.set_decay(1.0);
        d.set_release(0.0);
        d.sustain = 0.0;
        d.dec_shape = 0.0;
        d.att_shape = 0.0;
        d.dec_coef = (super::env::LN_EIGHTH / (d.dec_time * d.sr as f64)).exp();
        d.rel_shape = 0.0;
        d.rel_coef = 0.0;
        d.rel_time = (d.rel_raw + d.rel_raw) + f64::from_bits(0x3bc7_9ca1_0000_0000);
        d.inv_rel = 1.0 / d.rel_time;
        d.vel_sens = 0.0;
        d.gain = 1.0;
        d.sm_z = 0.0;
        d.sm_out = 0.0;
        if d.sm_hz != 20.0 {
            d.sm_hz = 20.0;
            let g = tan_approx((1.0 / sr) * f32::from_bits(0x427b_53d2));
            d.sm_g = g;
            d.sm_a = 1.0 / (g + 1.0);
            d.sm_b = g + -1.0;
        }
        let inv = 1.0 / sr;
        for p in [1, 2, 3, 0] {
            self.pd[p].set_sr(inv);
        }
        let gc = ((inv * NEG_LN2) / f32::from_bits(0x3ba3_d70a)).exp();
        let gc = if gc > 1.0 { 1.0 } else { gc };
        for gt in &mut self.gates {
            gt.state = 0;
            gt.v = 0.0;
            gt.coef = gc;
        }
        self.inv = inv;
        let q = (sr / 44100.0).sqrt();
        self.noise1 = q * f32::from_bits(0x3000_009d);
        self.noise2 = self.noise1;
        // The EQ chain.
        for bq in [&mut self.bq1, &mut self.bq2, &mut self.bq3, &mut self.bq4, &mut self.bq5] {
            bq.inv = inv;
            bq.clear();
        }
        self.bq3.bw = 4.0;
        self.bq4.bw = 2.5;
        let a4 = exp2_poly(self.bq4.gain * f32::from_bits(0x3daa_aa7e));
        self.bq4.k = clamp(f32::from_bits(0x4095_084b) / (a4 * f32::from_bits(0x4018_3958)), 0.01, 100.0);
        self.bq4.kk = a4.mul_add(a4, -1.0) * self.bq4.k;
        self.bq5.bw = 3.0;
        self.bq5.gain = 8.0;
        self.bq5.k = f32::from_bits(0x3fc7_7e84);
        self.bq5.kk = f32::from_bits(0x4017_abda);
        for op in [&mut self.op1, &mut self.op2, &mut self.op3, &mut self.op4, &mut self.op5] {
            op.inv = inv;
            op.z = 0.0;
        }
        self.op1.set_hz(5000.0);
        self.op2.set_hz(self.op2.hz);
        self.op3.set_hz(self.op3.hz);
        self.op4.set_hz(10.0);
        self.op5.set_hz(self.op5.hz);
        for s in [&mut self.svf1, &mut self.svf2] {
            s.sr = sr;
            s.inv = inv;
            let h = s.hz;
            s.set_hz(h);
            s.z1 = 0.0;
            s.z2 = 0.0;
        }
        self.bank.sr = sr;
        self.bank.inv = inv;
        let h = self.bank.hz;
        self.bank.set(h);
        self.bank.z1 = [0.0; 4];
        self.bank.z2 = [0.0; 4];
        for d in &mut self.dl {
            d.sr = sr;
            d.buf.fill(0.0);
        }
        self.dl[0].set_delay(f32::from_bits(0x3b03_126f));
        for c in [&mut self.comb1, &mut self.comb2] {
            c.sr = sr;
            c.buf.fill(0.0);
        }
        for s in [&mut self.sh1, &mut self.sh2] {
            s.inv = inv;
            s.inc = inv * s.rate;
            s.phase = 0.0;
        }
        self.out_gate.state = 0;
        let oc = ((inv * NEG_LN2) / f32::from_bits(0x3d4c_cccd)).exp();
        self.out_gate.coef = if oc > 1.0 { 1.0 } else { oc };
        self.reset();
    }

    /// `onReset()` (0x1018f1bbc); also the idle reset in `processAudio`.
    fn reset(&mut self) {
        self.vel = 0.0;
        self.played = false;
        self.first = true;
        for e in [&mut self.env_a, &mut self.env_b, &mut self.env_c, &mut self.env_d] {
            e.stage = 0;
            e.out = 0.0;
            e.x = 1.0;
            e.t = 0.0;
            e.sm_z = 0.0;
            e.sm_out = 0.0;
        }
        for p in &mut self.pd {
            p.v = 0.0;
        }
        for g in &mut self.gates {
            g.state = 0;
            g.v = 0.0;
        }
        self.phase = 0.0;
        for bq in [&mut self.bq1, &mut self.bq2, &mut self.bq3, &mut self.bq4, &mut self.bq5] {
            bq.clear();
        }
        for op in [&mut self.op1, &mut self.op2, &mut self.op3, &mut self.op4, &mut self.op5] {
            op.z = 0.0;
        }
        self.bank.z1 = [0.0; 4];
        self.bank.z2 = [0.0; 4];
        for s in [&mut self.svf1, &mut self.svf2] {
            s.z1 = 0.0;
            s.z2 = 0.0;
        }
        for d in &mut self.dl {
            d.buf.fill(0.0);
        }
        self.comb1.buf.fill(0.0);
        self.comb2.buf.fill(0.0);
        self.sh1.phase = 0.0;
        self.sh2.phase = 0.0;
        self.fb = 0.0;
        self.a_out = 0.0;
        self.lvl_a = 0.0;
        self.c_out = 0.0;
        self.lvl_c = 0.0;
        self.b_out = 0.0;
        self.lvl_b = 0.0;
        self.e_out = 0.0;
        self.lvl_e = 0.0;
        self.sm.cur = self.sm.target;
        self.sm.touch();
        self.out_gate.state = 0;
        self.out_gate.v = 0.0;
    }

    /// `setParamTune(v)` (0x1018f2230).
    fn set_tune(&mut self, v: f32) {
        let st = clamp(v, 0.0, 1.0).mul_add(24.0, 19.0) + self.note;
        self.st = st;
        let hz = exp_e(st.mul_add(5.0, PITCH));
        self.hz = hz;
        let t = &mut self.sm.target;
        t[ST] = st;
        t[HZ] = hz;
        let inv = 1.0 / hz;
        t[PERIOD] = inv;
        t[PITCH_ST] = st;
        let k = clamp((st + -23.0) / 14.0, 0.0, 1.0);
        t[CLICK] = k.mul_add(f32::from_bits(0x3ecc_cccc), f32::from_bits(0x3f19_999a)) * (inv * 3000.0);
        let r = 31.0 - st;
        let r = if r < 0.0 { 0.0 } else { r };
        t[SH_ST] = r.mul_add(f32::from_bits(0x3f4c_cccd), 71.0);
        t[SVF_HZ] = hz * 7.0;
        t[COMB_T] = 1.0 / (hz * f32::from_bits(0x4189_9581));
        self.decay_targets();
        self.bend_targets();
    }

    /// The targets set from Decay (and the pitch).
    fn decay_targets(&mut self) {
        let d = clamp(self.raw[1], 0.0, 1.0).mul_add(33.0, 32.0);
        let t = &mut self.sm.target;
        t[DECAY_T] = exp_e(d.mul_add(f32::from_bits(0x411f_73da), f32::from_bits(0x4416_8363)));
        let x = (self.st + -30.0).mul_add(f32::from_bits(0xbe99_999a), d + -4.0);
        t[B_REL] = exp_e(x.mul_add(f32::from_bits(0x411f_73da), f32::from_bits(0x4416_8363)));
    }

    /// The targets set from Bend (and the pitch, grind).
    fn bend_targets(&mut self) {
        let b = clamp(self.raw[2], 0.0, 1.0);
        let k = clamp((self.st + -25.0) / 10.0, 0.0, 1.0);
        let t = &mut self.sm.target;
        t[PD_AMT] = (1.0 - k).mul_add(15.0, b.mul_add(-6.0, 40.0));
        t[PD_T] = ((b.mul_add(f32::from_bits(0x3f66_6666), f32::from_bits(0x3dcc_cccd)) * f32::from_bits(0x3d23_d70a)) * 50.0) / self.hz;
        t[PD_LIM] = k.mul_add(-20.0, 50.0);
        t[EQ_GAIN] = b.mul_add(6.0, 8.0);
        self.grind_mod();
    }

    fn grind_mod(&mut self) {
        let g = clamp(self.raw[5], 0.0, 1.0).mul_add(25.0, 80.0);
        let b = clamp(self.raw[2], 0.0, 1.0).mul_add(f32::from_bits(0x3e99_999a), f32::from_bits(0x3f33_3333));
        self.sm.target[LP_MOD] = g * b;
        self.sm.touch();
    }

    /// `setParameter(i, v)` (0x1018f1de0): 0 tune, 1 decay, 2 bend, 3 impact, 4 gate,
    /// 5 aero grind, 6 aero amount.
    pub fn set(&mut self, i: usize, v: f32) {
        if i > 6 {
            return;
        }
        self.raw[i] = v;
        match i {
            0 => self.set_tune(v),
            1 => {
                self.decay_targets();
                self.sm.touch();
            }
            2 => self.bend_targets(),
            3 => {
                self.sm.target[IMPACT] = v.mul_add(v, v) * f32::from_bits(0x3f99_999a);
                self.sm.target[IMPACT_C] = clamp(v, 0.0, 1.0).mul_add(f32::from_bits(0x3f66_6666), f32::from_bits(0x3dcc_cccd));
                self.sm.touch();
            }
            4 => {
                let on = v > 0.0;
                self.gate = on;
                self.env_c.hold = on;
                self.env_a.hold = on;
                // The processing variants are re-chosen (the recomputing ones).
                self.first = true;
            }
            5 => {
                // setParamAeroGrind (0x1018f2588).
                let t = &mut self.sm.target;
                t[GRIND4] = clamp(v * 4.0, 0.0, 1.0);
                let s = clamp(v.abs().mul_add(f32::from_bits(0x3f4c_cccd), f32::from_bits(0x3e4c_cccc)) * v, 0.0, 1.0);
                t[GRIND_MIX] = s.mul_add(f32::from_bits(0x3f4c_cccd), f32::from_bits(0x3e4c_cccd));
                t[GRIND_DB] = clamp(v, 0.0, 1.0).mul_add(-12.0, 12.0);
                let a = clamp(v / f32::from_bits(0x3f19_999a), 0.0, 1.0);
                t[AERO_HZ] = exp_e(a.mul_add(70.0, 65.0).mul_add(5.0, PITCH));
                self.grind_mod();
            }
            _ => {
                self.sm.target[AERO] = v * 1.5;
                self.sm.touch();
            }
        }
    }

    /// `hasOutputSignal()` (0x1018f2bf0).
    pub fn active(&self) -> bool {
        let env = self.env_a.stage != 0 || self.env_b.stage != 0 || self.env_c.stage != 0;
        if !self.gate {
            return env;
        }
        let g = self.gates[1].state != 0 || self.gates[2].state != 0 || self.gates[0].state != 0;
        env && g
    }

    /// `onChoke()`: fade the output.
    pub fn choke(&mut self) {
        self.out_gate.release();
    }

    pub fn control(&mut self) {
        self.sm.control();
    }

    /// `setNoteOn(velocity, note)` (0x1018f2728).
    pub fn note_on(&mut self, vel: f32, note: u32) {
        if !(0.0..=1.0).contains(&vel) {
            self.vel = 0.0;
            return;
        }
        self.note = note as f32 + -60.0;
        if !self.active() {
            self.sm.cur = self.sm.target;
            self.sm.touch();
        }
        self.set_tune(self.raw[0]);
        let (c, t) = (&mut self.sm.cur, &self.sm.target);
        c[PITCH_ST] = t[PITCH_ST];
        c[SH_ST] = t[SH_ST];
        c[COMB_T] = t[COMB_T];
        c[B_REL] = t[B_REL];
        c[PERIOD] = t[PERIOD];
        c[PD_AMT] = t[PD_AMT];
        c[PD_LIM] = t[PD_LIM];
        self.env_a.note_on(vel);
        self.env_b.note_on(vel);
        self.env_c.note_on(vel);
        self.env_d.note_on(vel);
        self.pd[0].v = 1.0;
        self.pd[1].v = 1.0;
        let t = f32::from_bits(0x3983_126f);
        if self.pd[2].time != t {
            self.pd[2].time = t;
            self.pd[2].coef = Pd::coef_for(self.pd[2].inv, t);
        }
        self.pd[2].v = 1.0;
        self.pd[3].v = 1.0;
        for g in &mut self.gates {
            g.state = 1;
            g.v = 0.0;
        }
        self.out_gate.state = 1;
        self.out_gate.v = 0.0;
        let t = f32::from_bits(0x3ba3_d70a);
        self.dl[1].set_delay(t);
        self.dl[2].set_delay(t);
        self.phase = f32::from_bits(0x3f86_6666);
        self.seed2 = 10;
        let mx = |a: f32| if a > vel { a } else { vel };
        self.lvl_a = mx(self.a_out);
        self.lvl_c = mx(self.c_out);
        self.lvl_b = mx(self.b_out);
        self.lvl_e = mx(self.e_out);
        self.played = true;
        self.vel = vel;
        self.first = true;
    }

    /// `setNoteOff()` (0x1018f2f34): with Gate on, release the held envelopes and gates.
    pub fn note_off(&mut self) {
        if self.env_a.stage != 0 && self.env_a.stage < 4 && self.env_a.hold {
            while self.env_a.stage < 4 {
                self.env_a.advance();
            }
        }
        if self.env_c.stage != 0 && self.env_c.stage < 4 && self.env_c.hold {
            while self.env_c.stage < 4 {
                self.env_c.advance();
            }
        }
        for g in &mut self.gates {
            g.release();
        }
    }

    /// `processAudio` (0x1018f30e4) for one chunk; false when it wrote nothing.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) -> bool {
        let on = self.active();
        if on {
            let ramping = self.sm.ramping;
            let first = self.first;
            let gate = self.gate;
            for i in 0..l.len() {
                if ramping {
                    self.sm.tick();
                }
                if first || ramping {
                    self.recompute();
                }
                let y = self.sample(gate);
                l[i] = y;
                r[i] = y;
            }
            self.first = false;
        }
        if !self.active() && self.played {
            self.reset();
        }
        on
    }

    /// The per-sample coefficient updates of the recomputing variants.
    fn recompute(&mut self) {
        let c = self.sm.cur;
        self.env_a.set_decay(((c[DECAY_T] * 35.0) * c[PERIOD]) * f32::from_bits(0x3fe6_6666));
        self.env_b.set_release(c[B_REL]);
        let v = clamp(c[DECAY_T] * f32::from_bits(0x3e66_6666), -self.clip_c, self.clip_c);
        let v = v * (v.abs() * -0.25).mul_add(self.k_c, 1.0);
        self.env_c.set_decay(v);
        self.env_d.set_decay(c[DECAY_T] * f32::from_bits(0x3fe1_47ae));
        self.env_d.set_attack(c[DECAY_T] * f32::from_bits(0x3db4_3958));
        self.pd[0].set_time(c[PD_T]);
        self.pd[3].set_time(c[DECAY_T]);
        let hz = c[HZ];
        let b = &mut self.bq1;
        b.bw = 1.5;
        b.gain = -3.0;
        b.k = f32::from_bits(0x3fa5_8334);
        b.kk = f32::from_bits(0xbec1_cb44);
        b.set_fc(hz * 75.0);
        let b = &mut self.bq2;
        b.bw = 2.0;
        b.gain = 8.0;
        b.k = f32::from_bits(0x3f71_ded7);
        b.kk = f32::from_bits(0x3fb7_e39c);
        b.set_fc(hz * f32::from_bits(0x3fcc_cccd));
        self.op2.update(hz * 60.0);
        let h = hz * 1.5;
        self.bank.set([f32::from_bits(0x4582_d000), f32::from_bits(0x453b_8000), h, h]);
        self.bq4.update_fc(hz * 3.0);
        self.bq5.update_fc(hz * 85.0);
        let s = c[SVF_HZ];
        self.svf1.update(s * 0.75);
        self.op3.update(s);
        self.bq3.update_fc(s * 9.0);
        self.bq3.update_gain(c[EQ_GAIN]);
        self.svf2.update(c[AERO_HZ]);
        self.pd[1].set_time(c[PERIOD] * f32::from_bits(0x3e19_999a));
        self.comb1.set_delay(c[COMB_T]);
        self.comb2.set_delay(c[COMB_T]);
        let rate = exp_e((c[PITCH_ST] + c[SH_ST]).mul_add(5.0, PITCH));
        self.sh1.rate = rate;
        self.sh1.inc = rate * self.sh1.inv;
        self.sh2.rate = rate;
        self.sh2.inc = rate * self.sh2.inv;
        self.bq4.update_gain(c[GRIND_DB]);
    }

    /// One output sample (`processAudioImpl`, 0x1018ff778 / 0x1018f8f84 with Gate on).
    #[inline]
    fn sample(&mut self, gate: bool) -> f32 {
        let c = self.sm.cur;
        // Pitch: the sweep drop, soft-limited, onto the note.
        let p = self.pd[0].tick() * c[PD_AMT];
        let m = if c[PD_LIM] < f32::from_bits(0x2c0c_bccc) { f32::from_bits(0x2c0c_bccc) } else { c[PD_LIM] };
        let lim = m + m;
        let p = clamp(p, -lim, lim);
        let p = p * ((p.abs() * -0.25) / m + 1.0);
        let hz = exp_e((c[PITCH_ST] + p).mul_add(5.0, PITCH));
        let inc = clamp(hz * self.inv, -1.0, 1.0);
        // The phase, folded to a triangle, then a sine of it.
        let t = self.phase + inc + 10.0;
        let t = t - ((t + 0.5) as i32) as f32;
        self.phase = t;
        let u = t + 10.0;
        let u = u - ((u + 0.5) as i32) as f32;
        let tri = (u + u).abs() + -0.5;
        let s1 = (tri * PI) * f32::from_bits(0x3e22_f981);
        let h = if s1 < 0.0 { -0.5f64 } else { 0.5 };
        let n = (h + s1 as f64) as i32;
        let x = ((s1 - n as f32) + (s1 - n as f32)) * PI;
        let x2 = x * x;
        let q = x2.mul_add(f32::from_bits(0xb2cd_4ca8), f32::from_bits(0x3638_b94f));
        let q = q.mul_add(x2, f32::from_bits(0xb950_0c03));
        let q = q.mul_add(x2, f32::from_bits(0x3c08_8885));
        let q = q.mul_add(x2, f32::from_bits(0xbe2a_aac1));
        let q = q.mul_add(x2, 1.0);
        let sine = x * q;
        // Envelopes (and with Gate on, the hold-gates).
        let a_out = self.lvl_a * self.env_a.process();
        self.a_out = a_out;
        let body = if gate { sine * (a_out * self.gates[2].tick()) } else { sine * a_out };
        let body = clamp(body, -self.clip_a, self.clip_a);
        self.seed1 = self.seed1.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        let n1 = self.noise1 * self.seed1 as f32;
        let drive = sine * f32::from_bits(0x3fa6_6666) + n1;
        let c_out = self.lvl_c * self.env_c.process();
        self.c_out = c_out;
        let g1 = if gate { self.gates[0].tick() } else { 1.0 };
        let x0 = clamp(drive, -1.0, 1.0);
        let b_out = self.lvl_b * self.env_b.process();
        self.b_out = b_out;
        let bb = if gate { b_out * self.gates[1].tick() } else { b_out };
        self.seed2 = self.seed2.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        let n2 = bb * (self.noise2 * self.seed2 as f32);
        // The SVF bank: body, noise, noise burst, feedback.
        let v = self.bank.process([x0, n1, n2, self.fb]);
        self.fb = v[2];
        let click = clamp(v[1] * 1000.0, -self.clip_b, self.clip_b);
        let pd1 = self.pd[1].tick();
        let pd2 = self.pd[2].tick();
        let shaped_a = body * (body.abs() * -0.25).mul_add(self.k_a, 1.0);
        let x3 = v[3];
        let shaped_b = click * (click.abs() * -0.25).mul_add(self.k_b, 1.0);
        let d1 = self.dl[0].push(pd2);
        let s1 = v[1] * (pd1 * 4.0);
        let d2 = self.dl[1].push(d1);
        let d3 = self.dl[2].push(d2);
        let m = d1 * f32::from_bits(0x3e4c_cccd);
        let m = if m < d2 { d2 } else { m };
        let hh = d3 * 0.5;
        let m = if m < hh { hh } else { m };
        let s0 = shaped_b.mul_add(m, s1) * (c[IMPACT] * self.vel);
        let lp1 = self.op1.lp(shaped_a * 1.5);
        let lp2 = self.op2.lp(s0 + lp1);
        let y1 = self.bq1.process(lp2);
        let s15 = self.bq1.kk.mul_add(y1, lp2);
        let y2 = self.bq2.process(s15);
        // Aero: the SVF, combs and sample-and-holds on the bank's fourth lane.
        let svf = self.svf1.process(x3);
        let fbk = self.env_d.process() * f32::from_bits(0x3f4c_cccd);
        let fbk = if fbk > f32::from_bits(0x3f7f_f972) { f32::from_bits(0x3f7f_f972) } else { fbk };
        let y = self.comb1.read();
        self.comb1.write(fbk.mul_add(y, x3));
        let mix1 = clamp(c[GRIND_MIX], 0.0, 1.0);
        let s2 = ((x3 + y) * 0.5 - x3).mul_add(mix1, x3);
        let held = self.sh1.tick(s2);
        let s5 = c[GRIND_MIX] * c[GRIND4];
        let s2 = (held - s2).mul_add(mix1, s2);
        let y = self.comb2.read();
        self.comb2.write(fbk.mul_add(y, s2));
        let mix2 = clamp(s5, 0.0, 1.0);
        let s1 = ((s2 + y) * 0.5 - s2).mul_add(mix2, s2);
        let held = self.sh2.tick(s1);
        let s4 = svf * f32::from_bits(0x3f33_3333);
        let s0 = (held - s1).mul_add(mix1, s1);
        let s0 = s4 + self.op3.hp(s0);
        let y3 = self.bq3.process(s0);
        let e = self.lvl_e * self.pd[3].tick();
        self.e_out = e;
        let lp = self.op4.lp(e);
        let x = (c[ST] + 12.0) + lp * c[LP_MOD];
        let f = exp_e(x.mul_add(5.0, PITCH));
        let f = if f > 18000.0 { 18000.0 } else { f };
        self.op5.update(f);
        let lp = self.op5.lp(self.bq3.kk.mul_add(y3, s0));
        let y4 = self.bq4.process(lp);
        let att = exp_e((c[GRIND_DB] * f32::from_bits(0xbeb3_3333)).mul_add(f32::from_bits(0x411f_73da), 1200.0));
        let s0 = self.bq4.kk.mul_add(y4, lp) * att;
        let sv = self.svf2.process(s0);
        let s0 = c[CLICK] * (c[AERO] * sv);
        let y5 = self.bq5.process(s0);
        let og = self.out_gate.tick();
        let ct = if gate { (((c_out * g1) * 0.1) * c[IMPACT_C]) * v[0] } else { ((c_out * 0.1) * c[IMPACT_C]) * v[0] };
        let body = self.bq2.kk.mul_add(y2, s15);
        let aero = self.bq5.kk.mul_add(y5, s0);
        (((ct + body) + aero) * f32::from_bits(0x3e54_ee7c)) * og
    }
}
