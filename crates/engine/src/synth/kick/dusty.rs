//! The Kick's **Dusty** engine: `NI::DSP::DustyKickCore` (docs/synth-kick.md §4.3). Two
//! pulse oscillators (saw differences) and a parabolic triangle, swept by two pitch
//! envelopes, through a resonant state-variable filter and a soft clip; an attack module
//! (two sine clicks and a noise "knack"); a bank of four state-variable filters (NEON,
//! reciprocals by `FRECPE` + two Newton steps) for the 30 Hz high-pass of the body, the
//! noise band and a low resonance. Field comments give the member offset in
//! `DustyKickCore`; operations follow the assembly of `processAudioImpl<…>`
//! (0x1018e054c, 0x1018e134c, 0x1018e21b8, 0x1018e2f4c).

use super::env::{Decay, FreeEnv, Release};
use super::simd::recip;
use super::smooth::Smooth;
use super::sub::{exp2_poly, log_time, pitch_hz, tan_approx};
use crate::fx::exp_e;

const PI: f32 = std::f32::consts::PI;

/// Smoothed values (+0x900 current, +0x940 step, +0x980 target).
const DEC_C: usize = 0;
const DEC_D: usize = 1;
const DEC_A: usize = 2;
const REL_B: usize = 3;
const BASE: usize = 4;
const FILT: usize = 5;
const PITCH: usize = 6;
const TOP: usize = 7;
const RES_HZ: usize = 8;
const BAND_HZ: usize = 9;
const NOISE: usize = 10;
const IMPACT: usize = 11;
const KNACK: usize = 12;

/// A soft clipper (linear below `thr`, then `thr + d·(1 − sat·d/4)` up to `2·knee`).
#[derive(Clone, Copy, Debug)]
struct Clip {
    thr: f32,
    knee: f32,
    sat: f32,
    gain: f32,
}

impl Clip {
    #[inline]
    fn side(&self, x: f32) -> f32 {
        if self.thr <= x {
            let d = x - self.thr;
            let k = self.knee + self.knee;
            let d = if k < d { k } else { d };
            self.gain * d.mul_add((d * -0.25).mul_add(self.sat, 1.0), self.thr)
        } else {
            self.gain * x
        }
    }
    #[inline]
    fn both(pos: &Clip, neg: &Clip, x: f32) -> f32 {
        if x > 0.0 { pos.side(x) } else { neg.side(-x) }
    }
}

/// A scalar state-variable filter (TPT).
#[derive(Clone, Copy, Default, Debug)]
struct Svf {
    z1: f32,
    z2: f32,
    g: f32,
    k: f32,
    hz: f32,
    d: f32,
}

impl Svf {
    fn set(&mut self, sr: f32, inv: f32, hz: f32) {
        let hz = if hz < 0.0 { 0.0 } else { hz };
        let lim = sr * f32::from_bits(0x3efa_e148);
        let hz = if hz > lim { lim } else { hz };
        self.hz = hz;
        let g = tan_approx(inv * (hz * PI));
        self.g = g;
        self.d = 1.0 / g.mul_add(self.k + g, 1.0);
    }
    /// (high-pass, band-pass, low-pass)
    #[inline]
    fn process(&mut self, x: f32) -> (f32, f32, f32) {
        let (g, k, z1, z2) = (self.g, self.k, self.z1, self.z2);
        let s4 = z1 + z1;
        let s6 = z2.mul_add(2.0, g * s4);
        let gg = g.mul_add(k, g * g);
        let s4 = s4.mul_add(k, s6);
        let hp = x - self.d * x.mul_add(gg, s4);
        let v1 = g.mul_add(hp, z1);
        let bp = z1 + v1;
        let v2 = g.mul_add(bp, z2);
        self.z1 = v1;
        self.z2 = v2;
        (hp, bp, z2 + v2)
    }
}

/// Four state-variable filters in NEON lanes (+0x750…+0x7e4).
#[derive(Clone, Copy, Debug)]
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
    /// Cutoffs → coefficients (`fmin`/`fmax`, the series with the vector constants).
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
    /// One sample per lane (unfused): (high-pass, band-pass, low-pass).
    #[inline]
    fn process(&mut self, x: [f32; 4]) -> ([f32; 4], [f32; 4], [f32; 4]) {
        let mut hp = [0f32; 4];
        let mut bp = [0f32; 4];
        let mut lp = [0f32; 4];
        for i in 0..4 {
            let (g, k, z1, z2) = (self.g[i], self.k[i], self.z1[i], self.z2[i]);
            let a = z1 + z1;
            let b = (z2 + z2) + g * a;
            let gg = g * g + k * g;
            let c = k * a + b;
            let h = x[i] - self.d[i] * (x[i] * gg + c);
            let v1 = z1 + g * h;
            let p = z1 + v1;
            let v2 = z2 + g * p;
            hp[i] = h;
            bp[i] = p;
            lp[i] = z2 + v2;
            self.z1[i] = v1;
            self.z2[i] = v2;
        }
        (hp, bp, lp)
    }
}

/// A sine click of the attack module.
#[derive(Clone, Copy, Debug, Default)]
struct Click {
    width: f32,
    inc: f32,
    gain: f32,
    scale: f32,
    sens: f32,
    level: f32,
    active: bool,
    phase: f32,
}

impl Click {
    /// The click's half-sine at `x` (`w + phase`).
    #[inline]
    fn wave(&self, x: f32) -> f32 {
        let t = (x + -0.25) + 10.0;
        let t = t - ((t + 0.5) as i32 as f32);
        let t = (t + t) * PI * f32::from_bits(0x3e22_f981);
        let t = 0.5 - (t + t).abs();
        let t2 = t * t;
        let p = t2.mul_add(f32::from_bits(0x4010_fda0), f32::from_bits(0xc0a4_3f68));
        let p = t2.mul_add(p, f32::from_bits(0x4049_0fd0));
        let y = self.gain * (t * p);
        let m = f32::from_bits(0x3f7f_be77);
        y.clamp(-m, m)
    }
}

/// `UTIL::NoiseKnack` (+0x180): a noise-jittered oscillator through an envelope and a
/// 100 Hz high-pass.
#[derive(Clone, Debug)]
struct Knack {
    rate: f32,
    jitter: f32,
    clip: f32,
    gain: f32,
    out_gain: f32,
    seed_reset: i32,
    hold: bool,
    limit: f32,
    phase: f32,
    cnt: f32,
    cnt_inc: f32,
    env: FreeEnv,
    scale: f32,
    seed: i32,
    hp_z: f32,
    hp_a: f32,
    hp_b: f32,
}

impl Knack {
    fn process(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        if self.cnt < self.limit * f32::from_bits(0x3a83_126f) {
            self.cnt += self.cnt_inc;
            return 0.0;
        }
        let a = self.rate * (f32::from_bits(0x3fd5_3f7d) - self.jitter);
        let n = self.scale * self.seed as f32;
        let n = n + n;
        let j = self.jitter.clamp(0.0, 1.0);
        let step = self.rate * (a - n).mul_add(j, n);
        let p = self.phase + step;
        let t = (p + 1.0) * 0.25 + 10.0;
        let t = (t - ((t + 0.5) as i32 as f32)).abs() * 4.0;
        let tri = t + -1.0;
        self.phase = if tri.abs() < f32::from_bits(0x3727_c5ac) { tri } else { p };
        let e = self.env.process();
        let y =
            if self.hold { self.gain * (tri * e).clamp(-self.clip, self.clip) } else { e * (self.gain * tri.clamp(-self.clip, self.clip)) };
        let z = self.hp_a * (-self.hp_b).mul_add(self.hp_z, y);
        let h = z - self.hp_z;
        self.hp_z = z;
        let m = f32::from_bits(0x3f7f_be77);
        self.out_gain * h.clamp(-m, m)
    }
}

pub struct Dusty {
    /// +0x80…: raw parameters (0 tune, 1 decay, 2 filter, 3 impact, 4 noise).
    raw: [f32; 5],
    /// +0x94 pitch, +0x98 filter, +0x9c top pitch; +0xa4 note.
    pitch: f32,
    filt: f32,
    top: f32,
    note: u32,
    /// +0xa0 the bank's feedback sample; +0xa8 envelope A × start level, +0xac start.
    fb: f32,
    a_out: f32,
    a_start: f32,
    /// +0xb0/+0xb4 the first pulse's phases, +0xc0 their offset, +0xc4 1/sr; +0xc8/+0xcc,
    /// +0xd8, +0xdc the second's; +0xe0 the triangle's phase, +0xec 1/sr.
    p1: [f32; 2],
    off1: f32,
    inv_sr: f32,
    p2: [f32; 2],
    off2: f32,
    p3: f32,
    /// +0xf4 noise scale, +0xfc the noise generator.
    noise_scale: f32,
    seed: i32,
    /// +0x128: the attack module (enables, two clicks, the knack).
    en: [bool; 3],
    c1: Click,
    c2: Click,
    knack: Knack,
    /// +0x300: the velocity for the clicks.
    am_vel: f32,
    /// +0x308 A (body), +0x400 B (level), +0x4f8 C (pitch), +0x5f0 D (filter) envelopes.
    env_a: FreeEnv,
    env_b: FreeEnv,
    env_c: FreeEnv,
    env_d: FreeEnv,
    svf1: Svf,
    svf4: Svf,
    bank: Bank,
    sr: f32,
    /// +0x7fc / +0x840: the body and click shapers; +0x82c click clip, +0x830 its sat;
    /// +0x870 attack clip, +0x874 its sat.
    sh1: (Clip, Clip),
    sh2: (Clip, Clip),
    clk_clip: f32,
    clk_sat: f32,
    att_clip: f32,
    att_sat: f32,
    /// +0x878…: the level follower and fade.
    fol_decay: f32,
    fol_on: bool,
    fol_v: f32,
    fol_coef: f32,
    fol_out: f32,
    fol_held: f32,
    fol_mix: f32,
    fol_inv: f32,
    fol_thr: f32,
    fol_active: bool,
    fade_len: u32,
    fade_inv: f32,
    fade_n: u32,
    /// +0x8b0…: the anticlick crossfade (100 Hz).
    xf_pos: f32,
    xf_acc: f32,
    xf_now: f32,
    xf_out: f32,
    xf_prev: f32,
    xf_step: f32,
    xf_start: f32,
    xf_rate: f32,
    /// +0x8d8: the gate.
    gate: u32,
    gate_v: f32,
    gate_coef: f32,
    sm: Smooth<16>,
    /// +0x9e0 velocity, +0x9e4 played.
    vel: f32,
    played: bool,
    /// The recomputing `processAudioImpl` variants are in use (cleared by a ramp).
    first: bool,
}

impl Dusty {
    /// `DustyKickCore()` then `onSetSampleRate(sr, 128)`.
    pub fn new(sr: f32) -> Dusty {
        let mk = |sus: f32, dec_shape: f64, rel: Release| {
            let mut e = FreeEnv::new(Decay::Shape, rel);
            e.sustain = sus;
            e.dec_shape = dec_shape;
            e.vel_sens = 0.0;
            if rel == Release::Log {
                e.rel_shape = -1.0;
            }
            e
        };
        let env_a = mk(0.0, -f64::from(f32::from_bits(0x3e4c_cccd)), Release::Lin);
        let env_b = mk(f32::from_bits(0x3e05_1eb8), -f64::from(f32::from_bits(0x3e4c_cccd)), Release::Log);
        let env_c = mk(0.0, -f64::from(f32::from_bits(0x3f4c_cccd)), Release::Lin);
        let env_d = mk(0.0, -f64::from(f32::from_bits(0x3f4c_cccd)), Release::Lin);
        let mut kenv = FreeEnv::new(Decay::Lin, Release::Log);
        kenv.sustain = f32::from_bits(0x3e4c_cccd);
        kenv.vel_sens = 0.0;
        let half = Clip { thr: 0.0, knee: 0.5, sat: 2.0, gain: 1.0 };
        let hard = Clip { thr: 0.1, knee: f32::from_bits(0x2c0c_bccc), sat: f32::from_bits(0x52e8_d4a5), gain: 1.0 };
        let mut d = Dusty {
            raw: [0.0; 5],
            pitch: 0.0,
            filt: 0.0,
            top: 0.0,
            note: 60,
            fb: 0.0,
            a_out: 0.0,
            a_start: 0.0,
            p1: [0.0; 2],
            off1: 1.5,
            inv_sr: 1.0,
            p2: [0.0; 2],
            off2: 1.5,
            p3: 0.0,
            noise_scale: f32::from_bits(0x3000_009d),
            seed: 0,
            en: [true; 3],
            c1: Click {
                width: 0.0,
                inc: 0.0,
                gain: exp_e(1269.0 + f32::from_bits(0x3f42_b000)),
                scale: f32::from_bits(0x3f63_d70a),
                sens: 0.0,
                ..Click::default()
            },
            c2: Click {
                width: 0.0,
                inc: 0.0,
                gain: exp_e(1244.0 + f32::from_bits(0x3f58_9800)),
                scale: f32::from_bits(0x3f45_1eb8),
                sens: 0.0,
                ..Click::default()
            },
            knack: Knack {
                rate: 1.0,
                jitter: f32::from_bits(0x3ec7_ae14),
                clip: 1.0,
                gain: 1.0,
                out_gain: 1.0,
                seed_reset: 0,
                hold: false,
                limit: 0.0,
                phase: 0.0,
                cnt: 0.0,
                cnt_inc: 1.0,
                env: kenv,
                scale: 1.0,
                seed: 0,
                hp_z: 0.0,
                hp_a: 0.0,
                hp_b: 0.0,
            },
            am_vel: 0.0,
            env_a,
            env_b,
            env_c,
            env_d,
            svf1: Svf { k: 2.0, hz: 1.0, ..Svf::default() },
            svf4: Svf { k: f32::from_bits(0x3fe6_6666), hz: 1.0, ..Svf::default() },
            bank: Bank {
                z1: [0.0; 4],
                z2: [0.0; 4],
                g: [0.0; 4],
                k: [2.0, f32::from_bits(0x3fe6_6666), 2.0, 2.0],
                hz: [1.0; 4],
                d: [0.0; 4],
                sr,
                inv: 1.0,
            },
            sr,
            sh1: (half, Clip { gain: -1.0, ..half }),
            sh2: (hard, Clip { gain: -1.0, ..hard }),
            clk_clip: 2.0,
            clk_sat: 1.0,
            att_clip: f32::from_bits(0x4059_999a),
            att_sat: f32::from_bits(0x3f16_9696),
            fol_decay: 0.0,
            fol_on: true,
            fol_v: 0.0,
            fol_coef: 0.0,
            fol_out: 0.0,
            fol_held: 0.0,
            fol_mix: f32::from_bits(0x3e4c_cccd),
            fol_inv: 1.0,
            fol_thr: 1e-4,
            fol_active: false,
            fade_len: 1,
            fade_inv: 1.0,
            fade_n: 0,
            xf_pos: 1.0,
            xf_acc: 0.0,
            xf_now: 0.0,
            xf_out: 0.0,
            xf_prev: 0.0,
            xf_step: 0.0,
            xf_start: 0.0,
            xf_rate: 0.0,
            gate: 0,
            gate_v: 0.0,
            gate_coef: 0.0,
            sm: Smooth::new(),
            vel: 0.0,
            played: false,
            first: true,
        };
        d.set_sample_rate(sr, 128);
        d
    }

    /// `onSetSampleRate(sr, block)` (0x1018de474).
    fn set_sample_rate(&mut self, sr: f32, block: i32) {
        self.sm.set_rate(sr, block);
        let inv = 1.0 / sr;
        self.inv_sr = inv;
        for e in [&mut self.env_a, &mut self.env_c, &mut self.env_d] {
            e.set_samplerate(sr);
        }
        self.env_a.set_attack(f32::from_bits(0x3b51_b717));
        self.env_a.set_release(0.0);
        self.env_b.set_samplerate(sr);
        self.env_b.set_attack(f32::from_bits(0x3b51_b717));
        self.env_b.set_decay(f32::from_bits(0x3c93_74bc));
        self.env_c.set_attack(f32::from_bits(0x3a3f_5d79));
        self.env_c.set_release(0.0);
        self.env_d.set_attack(f32::from_bits(0x3a3f_5d79));
        self.env_d.set_release(0.0);
        // Attack module: clicks at 2062 Hz and 163 Hz, the knack.
        self.c1.inc = inv * 2062.0;
        self.c1.phase = 0.0;
        self.c1.active = false;
        self.c1.level = 0.0;
        self.c2.inc = inv * 163.0;
        self.c2.phase = 0.0;
        self.c2.active = false;
        self.c2.level = 0.0;
        let k = &mut self.knack;
        k.cnt_inc = inv;
        k.seed_reset = 0;
        let g = tan_approx(inv * f32::from_bits(0x439d_1463));
        k.hp_a = 1.0 / (g + 1.0);
        k.hp_b = g + -1.0;
        k.hp_z = 0.0;
        k.scale = f32::from_bits(0x3000_009d);
        k.env.set_samplerate(sr);
        k.phase = 0.0;
        k.cnt = 0.0;
        k.rate = (inv * 44100.0).sqrt() * f32::from_bits(0x3df2_5a26);
        k.limit = 0.0;
        k.clip = f32::from_bits(0x3e5d_6a14);
        k.gain = f32::from_bits(0x4093_fe77);
        k.env.set_attack(exp_e(392.0 + f32::from_bits(0x3f45_7e00)));
        k.env.set_decay(exp_e(768.0 + f32::from_bits(0x3f7a_d400)));
        // Filters.
        self.sr = sr;
        self.svf1.set(sr, inv, self.svf1.hz);
        self.svf1.z1 = 0.0;
        self.svf1.z2 = 0.0;
        let f = self.svf4.hz;
        self.svf4.set(sr, inv, f);
        if self.svf4.hz != f32::from_bits(0x44cf_ab33) {
            let f = (sr * f32::from_bits(0x3efa_e148)).min(f32::from_bits(0x44cf_ab33));
            self.svf4.hz = f;
            let g = tan_approx(inv * (f * PI));
            self.svf4.g = g;
            self.svf4.d = 1.0 / g.mul_add(self.svf4.k + g, 1.0);
        }
        self.svf4.z1 = 0.0;
        self.svf4.z2 = 0.0;
        self.bank.sr = sr;
        self.bank.inv = inv;
        let hz = self.bank.hz;
        self.bank.set(hz);
        self.bank.z1 = [0.0; 4];
        self.bank.z2 = [0.0; 4];
        // Follower, anticlick, gate.
        self.fol_inv = inv;
        let n = (sr * f32::from_bits(0x3c75_c290)) as u32;
        self.fade_len = if n > 1 { n } else { 1 };
        self.fade_inv = 1.0 / self.fade_len as f32;
        self.fol_out = 0.0;
        self.fol_held = 0.0;
        self.fol_v = 0.0;
        self.fol_active = false;
        self.fade_n = 0;
        self.xf_pos = 1.0;
        self.xf_acc = 0.0;
        self.xf_now = 0.0;
        self.xf_out = 0.0;
        self.xf_prev = 0.0;
        self.xf_step = 0.0;
        self.xf_start = 0.0;
        let r = inv * 100.0;
        self.xf_rate = r + r;
        let q = (sr / 44100.0).sqrt();
        self.noise_scale = q * f32::from_bits(0x3000_009d);
        self.gate = 0;
        self.gate_v = 0.0;
        self.gate_coef = ((inv * f32::from_bits(0xbf31_7215)) / 0.005).exp().min(1.0);
        self.sm.touch();
    }

    fn pitch_freq(&mut self) {
        self.sm.target[RES_HZ] = pitch_hz(self.top.mul_add(f32::from_bits(0x3f66_6666), self.filt) + 36.0);
    }

    fn knack_rel(&mut self) {
        let a = self.raw[1].mul_add(15.0, 35.0) + -3.0;
        let n = self.raw[4];
        let n = if n > 1.0 { 1.0 } else { n };
        let n = if n < 0.0 { 1.0 } else { n };
        let b = n.mul_add(23.0, 17.0);
        self.sm.target[KNACK] = if a < b { a } else { b };
    }

    /// `setParameter(i, v)` (0x1018df64c): 0 tune, 1 decay, 2 filter, 3 impact, 4 noise.
    pub fn set(&mut self, i: usize, v: f32) {
        if i > 4 {
            return;
        }
        self.raw[i] = v;
        match i {
            0 => {
                let n = self.note as f32 + -60.0;
                let b = (v.mul_add(12.0, 12.0) + n).clamp(12.0, 24.0);
                self.sm.target[BASE] = b;
                let p = (v.clamp(0.0, 1.0).mul_add(25.0, 30.0) + n).clamp(30.0, 55.0);
                self.pitch = p;
                self.sm.target[PITCH] = p;
                self.top = p + self.raw[2].clamp(0.0, 1.0).mul_add(30.0, -20.0);
                self.sm.target[TOP] = self.top;
                self.pitch_freq();
            }
            1 => {
                let d = v.mul_add(15.0, 51.0);
                self.sm.target[DEC_C] = log_time(d);
                self.sm.target[DEC_D] = log_time(d + 6.0);
                self.sm.target[DEC_A] = log_time(d + -3.0);
                let r = v.mul_add(15.0, 35.0);
                self.sm.target[REL_B] = log_time(if r > 41.0 { 41.0 } else { r });
                self.knack_rel();
            }
            2 => {
                let f = v.clamp(0.0, 1.0).mul_add(10.0, 35.0);
                self.sm.target[FILT] = f;
                self.filt = f;
                self.top = self.pitch + self.raw[2].clamp(0.0, 1.0).mul_add(30.0, -20.0);
                self.sm.target[TOP] = self.top;
                self.pitch_freq();
                self.sm.target[BAND_HZ] = pitch_hz(f.mul_add(1.5, -10.0));
            }
            3 => self.sm.target[IMPACT] = v + v,
            _ => {
                self.sm.target[NOISE] = v * 4.0;
                self.knack_rel();
            }
        }
        self.sm.touch();
    }

    /// Impact from the processor (`setParamImpact` case 2).
    pub fn set_impact(&mut self, v: f32) {
        self.raw[3] = v;
        self.sm.target[IMPACT] = v + v;
        self.sm.touch();
    }

    pub fn active(&self) -> bool {
        let on = self.env_a.stage != 0
            || self.fol_thr < self.fol_out
            || self.fol_active
            || (self.c1.active && self.en[0])
            || (self.c2.active && self.en[1])
            || (self.knack.env.stage != 0 && self.en[2]);
        on && self.gate != 0
    }

    pub fn choke(&mut self) {
        if self.gate == 1 {
            self.gate = 2;
            self.gate_v = 1.0;
        }
    }

    pub fn control(&mut self) {
        self.sm.control();
    }

    fn follower_coef(&self, t: f32) -> f32 {
        let lim = self.fol_inv * f32::from_bits(0x4a00_2c80);
        let t = if t < 1e-4 { 1e-4 } else { t };
        let t = if lim < t { lim } else { t };
        let c = exp2_poly((self.fol_inv * -3.0) / t);
        let m = f32::from_bits(0x3f7f_ff58);
        if c > m { m } else { c }
    }

    /// `setNoteOn(velocity, note)` (0x1018dfcf0).
    pub fn note_on(&mut self, vel: f32, note: u32) {
        if !(0.0..=1.0).contains(&vel) {
            self.vel = 0.0;
            return;
        }
        self.note = note;
        let t = self.raw[0];
        self.set(0, t);
        self.sm.cur = self.sm.target;
        self.sm.touch();
        self.vel = vel;
        self.p1 = [0.0, self.off1 + 0.0];
        self.p2 = [0.0, self.off2 + 0.0];
        self.p3 = 0.0;
        let c = self.sm.cur;
        self.env_a.set_decay(c[DEC_A]);
        self.env_a.note_on(vel);
        self.env_b.set_release(c[REL_B]);
        self.env_b.note_on(vel);
        self.env_c.set_decay(c[DEC_C]);
        self.env_c.note_on(vel);
        self.env_d.set_decay(c[DEC_D]);
        self.env_d.note_on(vel);
        self.am_vel = vel;
        self.c1.phase = 0.0;
        self.c1.active = true;
        self.c1.level = self.c1.scale * (vel + -1.0).mul_add(self.c1.sens.clamp(0.0, 1.0), 1.0);
        self.c2.phase = 0.0;
        self.c2.active = false;
        self.c2.level = 0.0;
        self.knack.phase = 0.0;
        self.knack.cnt = 0.0;
        self.knack.env.note_on(vel);
        if self.knack.seed_reset != 0 {
            self.knack.seed = self.knack.seed_reset;
        }
        self.fol_v = 1.0;
        self.fol_held = self.fol_out;
        if !self.fol_on {
            self.fol_coef = self.follower_coef(self.fol_decay);
        }
        self.fol_active = true;
        self.fade_n = 0;
        self.xf_pos = 1.0;
        self.xf_acc = 0.0;
        self.xf_step = self.xf_out - self.xf_prev;
        self.xf_start = self.xf_now;
        self.a_start = if self.a_out <= vel { vel } else { self.a_out };
        self.gate = 1;
        self.gate_v = 0.0;
        self.played = true;
        self.first = true;
    }

    fn idle_reset(&mut self) {
        self.vel = 0.0;
        self.played = false;
        self.fb = 0.0;
        self.a_out = 0.0;
        self.a_start = 0.0;
        self.p3 = 0.0;
        self.p1 = [0.0, self.off1 + 0.0];
        self.p2 = [0.0, self.off2 + 0.0];
        for e in [&mut self.env_a, &mut self.env_b, &mut self.env_c, &mut self.env_d, &mut self.knack.env] {
            e.stage = 0;
            e.out = 0.0;
            e.x = 1.0;
            e.t = 0.0;
        }
        self.c1.phase = 0.0;
        self.c1.active = false;
        self.c1.level = 0.0;
        self.c2.phase = 0.0;
        self.c2.active = false;
        self.c2.level = 0.0;
        self.knack.phase = 0.0;
        self.knack.cnt = 0.0;
        self.am_vel = 0.0;
        self.svf1.z1 = 0.0;
        self.svf1.z2 = 0.0;
        self.svf4.z1 = 0.0;
        self.svf4.z2 = 0.0;
        self.bank.z1 = [0.0; 4];
        self.bank.z2 = [0.0; 4];
        self.fol_out = 0.0;
        self.fol_held = 0.0;
        self.fol_v = 0.0;
        self.fol_active = false;
        self.fade_n = 0;
        self.xf_pos = 1.0;
        self.xf_acc = 0.0;
        self.xf_now = 0.0;
        self.xf_out = 0.0;
        self.xf_prev = 0.0;
        self.xf_step = 0.0;
        self.xf_start = 0.0;
        self.gate = 0;
        self.gate_v = 0.0;
        self.sm.cur = self.sm.target;
        self.sm.touch();
        self.first = true;
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) -> bool {
        if !self.active() {
            if self.played {
                self.idle_reset();
            }
            return false;
        }
        let smoothing = self.sm.ramping;
        let first = self.first;
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            let y = self.sample(first, smoothing);
            *a = y;
            *b = y;
        }
        if smoothing && first {
            self.first = false;
        }
        true
    }

    /// The attack module (`UTIL::AttackModule::process`, 0x10197b4ec).
    fn attack(&mut self) -> f32 {
        let mut out = 0.0;
        if self.c1.active && self.en[0] {
            let c = &mut self.c1;
            let p = c.phase + c.inc;
            c.phase = p;
            let lim = 1.0 - c.width;
            let (lvl, x) = if p >= lim {
                c.phase = 0.0;
                c.active = false;
                c.level = 0.0;
                (0.0, lim)
            } else {
                (c.level, p)
            };
            out = lvl * c.wave(c.width + x);
            if p >= lim {
                let c2 = &mut self.c2;
                c2.phase = 0.0;
                c2.active = true;
                c2.level = c2.scale * (self.am_vel + -1.0).mul_add(c2.sens.clamp(0.0, 1.0), 1.0);
            }
        } else if self.c2.active && self.en[1] {
            let c = &mut self.c2;
            let p = c.phase + c.inc;
            c.phase = p;
            let lim = 1.0 - c.width;
            let (lvl, x) = if p >= lim {
                c.phase = 0.0;
                c.active = false;
                c.level = 0.0;
                (0.0, lim)
            } else {
                (c.level, p)
            };
            out = lvl * c.wave(c.width + x);
        }
        if self.knack.env.stage != 0 && self.en[2] {
            out += self.knack.process();
        }
        out
    }

    #[inline]
    fn sample(&mut self, first: bool, smoothing: bool) -> f32 {
        if smoothing {
            self.sm.tick();
        }
        let check = |sm: &Smooth<16>, i: usize| first || (smoothing && sm.step[i] != 0.0);
        let c = self.sm.cur;
        if check(&self.sm, DEC_A) {
            self.env_a.set_decay(c[DEC_A]);
        }
        if check(&self.sm, REL_B) {
            let d = c[REL_B];
            if self.fol_decay != d {
                self.fol_decay = d;
                if self.fol_on {
                    self.fol_coef = self.follower_coef(d);
                }
            }
            self.env_b.set_release(d);
        }
        if check(&self.sm, DEC_C) {
            self.env_c.set_decay(c[DEC_C]);
        }
        if check(&self.sm, DEC_D) {
            self.env_d.set_decay(c[DEC_D]);
        }
        let ea = self.env_a.process();
        self.a_out = ea * self.a_start;
        let eb = self.vel * self.env_b.process();
        let held = self.fol_v * self.fol_held;
        self.fol_v *= self.fol_coef;
        let m = if held < eb { eb } else { held };
        let mix = self.fol_mix.clamp(0.0, 1.0);
        let lvl = ((eb + held) - m).mul_add(mix, m);
        let lvl = if lvl < 0.0 { 0.0 } else { lvl };
        self.fol_out = lvl;
        let fade = if lvl < self.fol_thr {
            let n = if self.fade_n < self.fade_len {
                self.fade_n + 1
            } else {
                self.fol_active = false;
                self.fade_len
            };
            self.fade_n = n;
            let g = (n as f32).mul_add(-self.fade_inv, 1.0);
            g * (g * g)
        } else {
            1.0
        };
        let ec = self.env_c.process();
        let ed = self.env_d.process();
        let p1 = c[PITCH].mul_add(ec, c[BASE]);
        let f1 = pitch_hz(p1);
        let pn = ((p1 + -40.0) / 12.0).clamp(0.0, 1.0);
        let f2 = pitch_hz(c[TOP].mul_add(ed, c[FILT]));
        let inc1 = (f1 * self.inv_sr).clamp(-1.0, 1.0);
        let f1b = f1 * f32::from_bits(0x3fa7_ecab);
        let inc2 = (f1b * self.inv_sr).clamp(-1.0, 1.0);
        let a = self.p1[0] + inc1;
        let lanes = [a + 10.0, (a + self.off1) + 10.0];
        for (o, x) in self.p1.iter_mut().zip(lanes) {
            *o = x - ((x + 0.5) as i32 as f32);
        }
        let b = inc2 + self.p2[0];
        let lanes = [b + 10.0, (b + self.off2) + 10.0];
        for (o, x) in self.p2.iter_mut().zip(lanes) {
            *o = x - ((x + 0.5) as i32 as f32);
        }
        if self.svf1.hz != f2 {
            self.svf1.set(self.sr, self.inv_sr, f2);
        }
        let amp = pn.mul_add(f32::from_bits(0xbf19_999a), f32::from_bits(0x3fcc_cccd)) * f32::from_bits(0x3f66_6666);
        let x = (self.p1[0] - self.p1[1]) + (self.p2[0] - self.p2[1]);
        let (_, _, lp) = self.svf1.process(x + x);
        let shaped = Clip::both(&self.sh1.0, &self.sh1.1, lp * f32::from_bits(0x3f42_8f5c));
        let inc3 = (f1 * self.inv_sr).clamp(-1.0, 1.0);
        let t = self.p3 + inc3 + 10.0;
        let ti = (t + 0.5) as i32 as f32;
        self.p3 = t - ti;
        let tri = self.p3 * (t - ti).abs().mul_add(-16.0, 8.0);
        let body = tri.mul_add(amp, shaped);
        self.seed = self.seed.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        if check(&self.sm, KNACK) {
            let r = log_time(c[KNACK]);
            self.knack.env.set_release(r);
        }
        let at = self.attack() * 3.0;
        let (h4, b4, _) = self.svf4.process(at);
        let at = self.vel * (c[IMPACT] * h4.mul_add(f32::from_bits(0x3f33_3333), b4));
        let at = at.clamp(-self.att_clip, self.att_clip);
        let sat = (at.abs() * -0.25).mul_add(self.att_sat, 1.0);
        self.bank.set([f2, 30.0, c[BAND_HZ], c[RES_HZ]]);
        let inp = [body, self.fb, self.noise_scale * self.seed as f32, at * sat];
        let (hp, bp, lpb) = self.bank.process(inp);
        self.fb = lpb[0];
        // Anticlick on the high-passed body.
        let mut o = self.a_out * hp[1];
        self.xf_prev = self.xf_out;
        self.xf_now = o;
        if self.xf_pos > 0.0 {
            let p = self.xf_pos - self.xf_rate;
            self.xf_pos = p;
            let s = p + -0.5;
            let s = (-s).mul_add(s.abs(), s).mul_add(2.0, 0.5);
            let acc = self.xf_acc;
            self.xf_acc = acc + self.xf_step;
            o += (self.xf_start + acc) * s;
        }
        self.xf_out = o;
        let lf = lvl * fade;
        let ab = tri.abs();
        let k = if ab > 0.75 { (ab + -0.75) * 8.0 } else { 0.0 };
        let k = k.clamp(-self.clk_clip, self.clk_clip);
        let sh = Clip::both(&self.sh2.0, &self.sh2.1, lf);
        let gate = match self.gate {
            0 => 0.0,
            1 => 1.0,
            _ => {
                let v = self.gate_coef * self.gate_v;
                self.gate_v = v;
                if (v as f64) < super::env::END {
                    self.gate_v = 0.0;
                    self.gate = 0;
                    0.0
                } else {
                    v
                }
            }
        };
        let k = k * (k.abs() * -0.25).mul_add(self.clk_sat, 1.0) + f32::from_bits(0x3ecc_cccd);
        let n = lf.mul_add(f32::from_bits(0x3dcc_cccd), sh) * 6.0;
        let n = c[NOISE] * (k * (bp[2] * n));
        let out = lpb[3] + (o + n);
        out * f32::from_bits(0x3e82_274a) * gate
    }
}
