//! Percussion › Shaker: `NI::DSP::ShakerCore` (+ `ShakerPerformer`) (docs/synth-perc.md §6).
//!
//! Noise through a band-pass, mixed with four random "grains" (phasors that restart at a
//! random rate and set a random decaying level), then three state-variable filters in
//! series (cutoffs from the Tune/Filter parameters), an envelope follower that normalises
//! the level, and one of three envelopes (Env Mode): Machine (lin attack / hold / shaped
//! release + a decaying level block), Realistic (inverted-shape attack, log decay), or
//! Performer (a tempo-synced shake pattern with swing, fill, accent and spread).
//!
//! As the other cores, the state is a byte image of the C++ object.

use super::dsp::env::Kind::{InvLog, LinShape};
use super::dsp::{self, Mem, clamp01, decay_coef, env, exp_table, exp2_s, smooth, svf};

const SIZE: usize = 0xa00;

const P: usize = 0x80; // parameters as last set, 17 floats
const PITCH: usize = 0xc4;
const MODE: usize = 0xc8; // Env Mode as set
const MODE_NOTE: usize = 0xcc; // Env Mode of the note playing
const LEVEL: usize = 0xd0; // 4 grain levels
const PHASE: usize = 0xe0; // 4 grain phases
const BASE: usize = 0xf0; // 4 base rates (90, 80, 75, 70 Hz)/sr
const RATE: usize = 0x100;
const RND: usize = 0x110;
const FOLLOW: usize = 0x124;
const NOISE: usize = 0x134; // flag, scale +4, sr +8, seed +0xc
const NSVF: usize = 0x16c;
const SVF: [usize; 3] = [0x19c, 0x1cc, 0x1fc];
const HP12K: usize = 0x22c;
const GDEC: usize = 0x24c; // 4 grain decays, stride 0x14: time, level, coef, 1/sr, floor
const ENV_R: usize = 0x2a0; // Realistic
const ENV_M: usize = 0x398; // Machine
const PERF: usize = 0x490;
const SB: usize = 0x590; // SmoothingToolbox<12>: performer values
const GRN_R: usize = 0x640; // 4 noise generators for the rates, stride 0x38
const GRN_L: usize = 0x720; // 4 for the levels
const BLK_R: usize = 0x800; // level block (Realistic)
const BLK_M: usize = 0x834; // level block (Machine)
const GATE: usize = 0x868;
const SA: usize = 0x890; // SmoothingToolbox<15> (16-lane layout)
const VEL: usize = 0x990;
const STARTED: usize = 0x994;
const SR: usize = 0x998;

/// `ShakerCore`.
pub struct Shaker {
    m: Mem,
    /// The tempo (BPM) and the host transport (position 0, stopped).
    first: bool,
}

/// A rounding as the core converts enum values (`±0.5` in f64, truncate).
fn round_param(v: f32) -> i32 {
    let h = if v < 0.0 { -0.5 } else { 0.5 };
    (h + v as f64) as i32
}

impl Shaker {
    /// The constructor (0x101927a88).
    pub fn new() -> Shaker {
        let mut m = Mem::new(SIZE);
        let t = exp_table();
        m.set_u(0x130, 60);
        m.set_f(0x164, 1.0);
        m.set_f(0x168, 1.0);
        m.set_u(NOISE + 4, 0x3000_009d);
        m.set_f(NOISE + 8, 1.0);
        m.set_f(0x148, 1.0);
        for b in [NSVF, SVF[0], SVF[1], SVF[2]] {
            m.set_f(b + 0x28, 1.0);
            m.set_f(b + 0x2c, 1.0);
            m.set_q(b + 0x18, [0.5, 1.0, 0.75, 1.0]);
        }
        m.set_q(HP12K + 0x18, [1.0, 1.0, 1.0, 0.0]);
        let half = clamp01(t.at(1140.0));
        for i in 0..4 {
            let g = GDEC + 0x14 * i;
            m.set_f(g, 1.0);
            m.set_f(g + 4, 0.0);
            m.set_f(g + 8, half);
            m.set_f(g + 0xc, 1.0);
            m.set_f(g + 0x10, f32::from_bits(0x38d1_b717));
        }
        // Realistic envelope (FE<1,1,2,2,0>).
        for e in [ENV_R, ENV_M] {
            m.set_u(e, 0);
            m.set_d(e + 0x18, 1.0);
            for o in (0x58..=0x80).step_by(8) {
                m.set_d(e + o, 1.0);
            }
            m.set_d(e + 0x88, 1.0);
            m.set_f(e + 0xa8, 0.5);
            m.set_q(e + 0xb0, [20000.0, 1.0, 1.0, 1.0]);
            m.set_q(e + 0xd8, [1.0, 1.0, 0.0, 0.0]);
            m.set_f(e + 0xe8, 0.0);
            m.set_f(e + 0xec, -1.0);
            m.set_d(e + 0xf0, 1.0);
        }
        // Performer basics.
        m.set_x(PERF, 0x3f00_0000_0000_0000);
        m.set_f(PERF + 8, 0.5);
        m.set_u(PERF + 0x14, 0);
        m.set_u(PERF + 0xc, 0);
        m.set_q(PERF + 0x18, [0.5, 0.0, 0.0, -1.0]);
        m.set_d(PERF + 0x28, 1.0);
        m.set_x(PERF + 0x30, 0);
        m.set_b8(PERF + 0x38, 0);
        m.set_x(PERF + 0x3c, 0x3f80_0000_0000_0004);
        m.set_x(PERF + 0x44, 0x3e80_0000_0000_0004);
        m.set_u(PERF + 0x6c, 0);
        m.set_f(PERF + 0x70, 1.0);
        m.set_f(PERF + 0x78, 1.0);
        m.set_f(PERF + 0x7c, 0.0);
        m.set_f(PERF + 0xa0, 1.0);
        m.set_f(PERF + 0xa4, 1.0);
        m.set_q(PERF + 0xc0, [1.0, 1.0, -1.0, -1.0]);
        m.set_f(PERF + 0xe0, 1.0);
        for i in 0..4 {
            for g in [GRN_R + 0x38 * i, GRN_L + 0x38 * i] {
                m.set_f(g + 8, 1.0);
                m.set_f(g + 0x30, 1.0);
                m.set_f(g + 0x34, 1.0);
                m.set_u(g + 4, 0x3000_009d);
                m.set_f(g + 0x14, 1.0);
            }
        }
        // Level blocks.
        m.set_f(BLK_R, 1.0);
        m.set_f(BLK_R + 0x20, f32::from_bits(0x38d1_b717));
        m.set_f(BLK_R + 0x1c, 1.0);
        m.set_x(BLK_R + 0x28, 0x3f80_0000_0000_0001);
        m.set_x(BLK_M - 4, 0x3f80_0000_0000_0000);
        m.set_f(BLK_M + 0x1c, 1.0);
        m.set_f(BLK_M + 0x20, f32::from_bits(0x38d1_b717));
        m.set_x(BLK_M + 0x28, 0x3f80_0000_0000_0001);
        m.set_f(GATE + 0x14, 1.0);
        m.set_q(GATE + 4, [1.0, 1.0, 0.0, f32::from_bits(0x3f00_0001)]);
        m.set_f(SR, -1.0);
        m.set_u(SR + 4, u32::MAX);
        smooth::snap(&mut m, SA, 16);
        for b in [NSVF, SVF[0], SVF[1], SVF[2]] {
            svf::init_k(&mut m, b);
        }
        // Realistic: sustain 0, attack curve −0.6, linear release.
        m.set_f(ENV_R + 0xa8, 0.0);
        m.set_d(ENV_R + 0x90, -0.6f32 as f64);
        m.set_d(ENV_R + 0x98, 0.0);
        let e = (dsp::LN8_NEG / (m.d(ENV_R + 0x68) * m.f(ENV_R + 0xec) as f64)).exp();
        m.set_d(ENV_R + 0x40, e);
        m.set_d(ENV_R + 0xa0, 0.0);
        m.set_d(ENV_R + 0x48, 0.0);
        let r = m.d(ENV_R + 0x80);
        let r = (r + r) + f64::from_bits(0x3bc7_9ca1_0000_0000);
        m.set_d(ENV_R + 0x78, r);
        m.set_d(ENV_R + 0x88, 1.0 / r);
        m.set_f(ENV_R + 0xb4, 1.0);
        m.set_f(ENV_R + 0xb8, 0.0);
        // Machine: sustain 1, shaped release (curve 0.1).
        m.set_f(ENV_M + 0xa8, 1.0);
        m.zero(ENV_M + 0x90, 16);
        let sr = m.f(ENV_M + 0xec) as f64;
        m.set_d(ENV_M + 0x40, (dsp::LN8_NEG / (m.d(ENV_M + 0x68) * sr)).exp());
        m.set_d(ENV_M + 0xa0, f64::from_bits(0x3fb9_9999_a000_0000));
        m.set_d(ENV_M + 0x48, 0.0);
        let r = m.d(ENV_M + 0x80);
        let r = (r + r) / f64::from_bits(0x3fef_5cfa_ac88_ce71) + f64::from_bits(0x3bc7_9ca1_0000_0000);
        m.set_d(ENV_M + 0x78, r);
        m.set_d(ENV_M + 0x88, 1.0 / r);
        m.set_f(ENV_M + 0xb4, 1.0);
        m.set_f(ENV_M + 0xb8, 0.0);
        m.set_b8(BLK_M + 4, 1);
        m.set_f(BLK_M + 0x18, 0.0);
        m.set_b8(BLK_R + 4, 1);
        m.set_f(BLK_R + 0x18, 0.0);
        Shaker { m, first: true }
    }

    /// `onSetSampleRate(sr, period)` (0x101928280).
    pub fn set_sample_rate(&mut self, sr: f32, period: i32) {
        let m = &mut self.m;
        smooth::set_rate(m, SA, 16, sr, period);
        m.set_f(NOISE + 8, sr);
        let inv = 1.0 / sr;
        m.set_f(0x168, inv);
        one_pole(m, 0x14c, inv, 15.0, f32::from_bits(0x423c_7edd));
        m.set_b8(NOISE, 1);
        let s = (sr / 44100.0).sqrt();
        m.set_f(NOISE + 4, s * f32::from_bits(0x3000_009d));
        m.set_f(0x148, 1.0 / s);
        for b in [NSVF, SVF[0], SVF[1], SVF[2]] {
            svf::set_rate(m, b, sr, inv);
        }
        m.set_f(HP12K + 0x1c, inv);
        let f = m.f(HP12K + 0x18);
        if f == -1.0 {
            m.set_u(HP12K, 0);
            m.set_x(HP12K + 4, 0);
            hp_coefs(m, HP12K, inv, 12000.0, f32::from_bits(0x4713_431d));
        } else {
            let g = dsp::tan_scalar(inv * (f * std::f32::consts::PI));
            m.set_f(HP12K + 0x14, g);
            m.set_f(HP12K + 0xc, 1.0 / (g + 1.0));
            m.set_f(HP12K + 0x10, g + -1.0);
            m.set_u(HP12K, 0);
            m.set_x(HP12K + 4, 0);
            if f != 12000.0 {
                hp_coefs(m, HP12K, inv, 12000.0, f32::from_bits(0x4713_431d));
            }
        }
        for i in 0..4 {
            for g in [GRN_R + 0x38 * i, GRN_L + 0x38 * i] {
                m.set_f(g + 8, sr);
                one_pole(m, g + 0x18, inv, 15.0, f32::from_bits(0x423c_7edd));
                m.set_b8(g, 0);
                m.set_u(g + 4, 0x3000_009d);
                m.set_x(g + 0x14, 0x3f80_0000);
            }
        }
        let base = [90.0f32, 80.0, 75.0, 70.0].map(|x| x / sr);
        for i in 0..4 {
            m.set_f(BASE + 4 * i, base[i]);
            m.set_f(RATE + 4 * i, base[i] * (m.f(RND + 4 * i) + 1.0));
        }
        let n = (sr * f32::from_bits(0x3c75_c290)) as u32;
        let n = if n > 1 { n } else { 1 };
        let k = 1.0 / n as f32;
        for b in [BLK_R, BLK_M] {
            m.set_f(b + 0x1c, inv);
            m.set_u(b + 0x28, n);
            m.set_f(b + 0x2c, k);
            m.set_x(b + 0x10, 0);
            m.set_u(b + 8, 0);
            m.set_b8(b + 0x24, 0);
            m.set_u(b + 0x30, 0);
        }
        env::set_rate(m, ENV_R, sr, InvLog);
        if m.f(ENV_R + 0xe8) != 0.0 {
            m.set_f(ENV_R + 0xe8, 0.0);
            m.set_d(ENV_R + 0x80, 0.0);
            let c = m.d(ENV_R + 0xa0);
            if c == -1.0 {
                let e = (dsp::LN8_NEG / (m.f(ENV_R + 0xec) as f64 * 0.0)).exp();
                m.set_d(ENV_R + 0x48, e);
                if m.u(ENV_R + 0x10) == 4 {
                    m.set_d(ENV_R + 0x38, e);
                }
            } else {
                m.set_d(ENV_R + 0x48, 0.0);
                let q = (-c).mul_add(c, 1.0);
                let t = 0.0 / (q * q) + f64::from_bits(0x3bc7_9ca1_0000_0000);
                m.set_d(ENV_R + 0x78, t);
                let r = 1.0 / t;
                m.set_d(ENV_R + 0x88, r);
                if m.u(ENV_R + 0x10) == 4 {
                    m.set_d(ENV_R + 0x30, r * m.d(ENV_R + 0xf0));
                }
            }
        }
        env::set_rate(m, ENV_M, sr, LinShape);
        perf_set_sample_rate(m, sr, period);
        for i in 0..4 {
            let g = GDEC + 0x14 * i;
            m.set_f(g + 0xc, inv);
            m.set_f(g + 4, 0.0);
            m.set_f(g + 8, decay_coef(inv, m.f(g)));
        }
        let x = sr * f32::from_bits(0x3c23_d70a);
        let ln2 = f32::from_bits(0x3f31_7215);
        let c = if x < 1.0 { ln2 } else { ln2 / x };
        m.set_f(0x128, c);
        m.set_f(0x12c, c);
        m.set_f(GATE + 0x14, inv);
        m.set_u(GATE, 0);
        m.set_f(GATE + 8, f32::from_bits(0x3ba3_d70a));
        m.set_f(GATE + 0xc, 0.0);
        let e = ((inv * f32::from_bits(0xbf31_7215)) / f32::from_bits(0x3ba3_d70a)).exp();
        m.set_f(GATE + 0x10, if e > 1.0 { 1.0 } else { e });
        m.set_f(SR, sr);
        m.set_u(SR + 4, period as u32);
    }

    /// `onReset` (0x101929d04).
    pub fn reset(&mut self) {
        let m = &mut self.m;
        m.set_f(VEL, 0.0);
        m.set_b8(STARTED, 0);
        smooth::snap(m, SA, 16);
        for b in [NSVF, SVF[0], SVF[1], SVF[2]] {
            m.zero(b, 0x14);
        }
        m.zero(HP12K, 12);
        for i in 0..4 {
            m.set_f(GDEC + 0x14 * i + 4, 0.0);
        }
        m.set_u(ENV_R, 0);
        m.zero(ENV_R + 8, 16);
        m.set_x(ENV_R + 0x20, 0);
        m.set_d(ENV_R + 0x18, 1.0);
        m.set_x(ENV_R + 0xc0, 0);
        m.set_u(ENV_R + 0xc8, 0);
        m.set_u(ENV_M, 0);
        m.zero(ENV_M + 8, 16);
        m.set_d(ENV_M + 0x18, 1.0);
        m.set_d(ENV_M + 0x20, 0.0);
        m.set_x(ENV_M + 0xc0, 0);
        m.set_u(ENV_M + 0xc8, 0);
        m.set_x(BLK_R + 0x10, 0);
        m.set_u(BLK_R + 8, 0);
        m.set_b8(BLK_R + 0x24, 0);
        m.set_u(BLK_R + 0x30, 0);
        m.set_x(BLK_M + 0x10, 0);
        m.set_u(BLK_M + 8, 0);
        m.set_b8(BLK_M + 0x24, 0);
        m.set_x(BLK_M + 0x30, 0);
        m.set_u(GATE + 0xc, 0);
        m.set_u(PERF + 0xc, 0);
        m.set_x(PERF + 0x58, 0);
        m.set_x(PERF + 0x68, 0);
        m.set_u(PERF + 0x74, 0);
        smooth::snap(m, SB, 12);
        m.set_u(SB + 0x50, 0);
        m.set_u(SB + 0x80, 0);
        m.set_u(SB + 0x20, 0);
        m.set_u(PERF + 0xd0, 0);
        m.set_b8(PERF + 0xd4, 0);
        m.set_u(PERF + 0x7c, 0);
        m.set_u(PERF + 0x90, 0);
        m.set_x(PERF + 0x88, 0);
        m.set_u(PERF + 0x84, 0);
    }

    /// `setParameter(idx, v)` (0x101929ec8).
    pub fn set_param(&mut self, idx: u32, v: f32) {
        let m = &mut self.m;
        if idx > 16 {
            return;
        }
        m.set_f(P + 4 * idx as usize, v);
        let t = exp_table();
        match idx {
            0 => {
                let p = clamp01(v).mul_add(80.0, 55.0);
                let n = m.u(0x130) as f32 + -60.0;
                let p = p + n;
                let p = if p < 55.0 { 55.0 } else { p };
                let p = if p > 135.0 { 135.0 } else { p };
                m.set_f(PITCH, p);
                m.set_f(SA + 0x80 + 0x30, p);
                grain_lanes(m, m.f(P + 4), p);
                filter_lanes(m, m.f(P + 8), p);
            }
            1 => {
                let c = clamp01(v);
                m.set_f(SA + 0x80 + 0x24, c + f32::from_bits(0x3f33_3333));
                m.set_f(SA + 0x80 + 0x28, c.mul_add(f32::from_bits(0x3f4c_cccd), 0.0));
                let g = m.f(P + 4);
                m.set_f(SA + 0x80 + 0x2c, g);
                grain_lanes(m, g, m.f(PITCH));
                smooth::start(m, SA, 16);
            }
            2 => {
                filter_lanes(m, m.f(P + 8), m.f(PITCH));
            }
            3 => m.set_u(MODE, round_param(v) as u32),
            4 => {
                let c = clamp01(v);
                m.set_f(SA + 0x80, t.db60(c.mul_add(30.0, 15.0)));
                m.set_f(SA + 0x80 + 0xc, t.db60(c.mul_add(40.0, 15.0)));
                smooth::start(m, SA, 16);
            }
            5 => {
                let c = clamp01(v);
                m.set_f(SA + 0x80 + 4, t.db60(c.mul_add(f32::from_bits(0x420c_0000), 5.0)));
                smooth::start(m, SA, 16);
            }
            6 => {
                let c = clamp01(v);
                m.set_f(SA + 0x80 + 8, t.db60(c.mul_add(30.0, 15.0)));
                m.set_f(SA + 0x80 + 0x10, t.db60(c.mul_add(40.0, 15.0)));
                smooth::start(m, SA, 16);
            }
            7 => perf_set_accent(m, v),
            8 => perf_set_spread(m, v),
            9 => pf_fill(m, round_param(v)),
            10 => {
                let n = round_param(v);
                if m.u(PERF + 0x3c) as i32 != n {
                    restart_pattern(m);
                }
                m.set_u(PERF + 0x3c, n as u32);
                let r = RATES[(n as u32 as usize).min(7)];
                m.set_f(PERF + 0x40, r);
                let d = (r as f64 + r as f64) * f64::from_bits(0x3f85_5555_5555_5555);
                m.set_d(PERF + 0x60, 1.0 / d);
                let bps = m.d(PERF + 0x28);
                m.set_d(PERF + 0x50, bps / (d * m.f(PERF + 0x24) as f64));
            }
            11 => {
                m.set_f(PERF + 0x1c, v);
                let (a, b) = if m.b8(PERF + 0x11) != 0 {
                    (0.5, 0.5)
                } else {
                    let s = v * v.abs().mul_add(0.5, 0.5);
                    let a = clamp01(s).mul_add(0.25, 0.5);
                    let c = clamp01((s + f32::from_bits(0xbe4c_cccd)) / f32::from_bits(0x3f4c_cccd));
                    let tw = m.f(PERF + 0x20);
                    let k = f32::from_bits(0x3e19_999a);
                    let b = a - c.mul_add(k, k) * tw;
                    if m.b8(PERF + 0x12) == 0 { (a, b) } else { (0.5, 0.5) }
                };
                m.set_f(SB + 0x60 + 0x24, a);
                m.set_f(SB + 0x60 + 0x28, b);
                smooth::start(m, SB, 12);
            }
            12 => {
                m.set_f(PERF + 0x20, v);
                let b = if m.b8(PERF + 0x11) != 0 {
                    0.5
                } else {
                    let sw = m.f(PERF + 0x1c);
                    let s = sw * sw.abs().mul_add(0.5, 0.5);
                    let k = f32::from_bits(0x3e19_999a);
                    let c = clamp01((s + f32::from_bits(0xbe4c_cccd)) / f32::from_bits(0x3f4c_cccd)).mul_add(k, k);
                    let a = clamp01(s).mul_add(0.25, 0.5);
                    let b = a - c * v;
                    if m.b8(PERF + 0x12) == 0 { b } else { 0.5 }
                };
                m.set_f(SB + 0x60 + 0x28, b);
                smooth::start(m, SB, 12);
            }
            13 => {
                let n = round_param(v);
                if m.u(PERF + 0x44) as i32 != n {
                    m.set_u(PERF + 0x44, n as u32);
                    restart_pattern(m);
                    m.set_f(PERF + 0x48, 1.0 / n as f32);
                }
            }
            14 => {
                let n = round_param(v);
                if m.u(PERF + 0x14) as i32 != n {
                    m.set_u(PERF + 0x14, n as u32);
                    restart_pattern(m);
                }
            }
            15 => {
                m.set_f(SB + 0x60 + 0x2c, v);
                smooth::start(m, SB, 12);
            }
            16 => {
                let on = (v > 0.5) as u8;
                let was = m.b8(PERF + 0x10);
                if was != on {
                    restart_pattern(m);
                }
                if was != 0 {
                    let d = m.d(PERF + 0x30).mul_add(m.d(PERF + 0x60), m.d(PERF + 0x58));
                    m.set_d(PERF + 0x58, d);
                }
                m.set_x(PERF + 0x30, 0);
                m.set_b8(PERF + 0x10, on);
            }
            _ => {}
        }
    }

    /// `setNoteOn(velocity, note)` (0x10192b1f0).
    pub fn note_on(&mut self, vel: f32, note: u32) {
        let m = &mut self.m;
        if !(vel >= 0.0 && vel <= 1.0) {
            m.set_f(VEL, 0.0);
            return;
        }
        m.set_u(0x130, note);
        let p = clamp01(m.f(P)).mul_add(80.0, 55.0);
        let p = (note as f32 + -60.0) + p;
        let p = if p < 55.0 { 55.0 } else { p };
        let p = if p > 135.0 { 135.0 } else { p };
        m.set_f(PITCH, p);
        m.set_f(SA + 0x80 + 0x30, p);
        grain_targets(m, m.f(P + 4), p);
        let c = clamp01(m.f(P + 8));
        m.set_f(SA + 0x80 + 0x34, (142.0 - p).mul_add(c, p));
        m.set_f(SA + 0x80 + 0x38, p - c.mul_add(36.0, 0.0));
        smooth::snap(m, SA, 16);
        let (want, cur) = (m.u(MODE), m.u(MODE_NOTE));
        let mut want = want;
        if (want == 2 || cur == 2) && cur != want {
            self.reset();
            want = self.m.u(MODE);
        }
        let m = &mut self.m;
        m.set_u(MODE_NOTE, want);
        if m.f(ENV_R + 0xec) > 0.0 {
            env::set_attack(m, ENV_R, m.f(SA + 0xc));
        }
        if m.f(ENV_M + 0xec) > 0.0 {
            env::set_attack(m, ENV_M, m.f(SA));
        }
        match m.u(MODE) {
            0 => {
                m.set_f(BLK_M + 8, 1.0);
                m.set_f(BLK_M + 0x14, m.f(BLK_M + 0x10));
                if m.b8(BLK_M + 4) == 0 {
                    let t = m.f(BLK_M);
                    block_coef(m, BLK_M, t);
                }
                m.set_b8(BLK_M + 0x24, 1);
                m.set_u(BLK_M + 0x30, 0);
                env::note_on(m, ENV_M, vel, LinShape);
            }
            1 => {
                let t = m.f(0x120);
                let changed = m.f(BLK_R) != t;
                m.set_f(BLK_R + 8, 1.0);
                m.set_f(BLK_R + 0x14, m.f(BLK_R + 0x10));
                if changed {
                    m.set_f(BLK_R, t);
                    if m.b8(BLK_R + 4) != 0 {
                        block_coef(m, BLK_R, t);
                    }
                }
                if m.b8(BLK_R + 4) == 0 {
                    block_coef(m, BLK_R, t);
                }
                m.set_b8(BLK_R + 0x24, 1);
                m.set_u(BLK_R + 0x30, 0);
                env::note_on(m, ENV_R, vel, InvLog);
            }
            2 => {
                m.set_f(PERF + 0xc, 1.0);
                m.set_u(PERF, m.u(PERF).wrapping_add(1));
                if m.b8(PERF + 0x10) == 0 || m.b8(PERF + 0x38) == 0 {
                    m.set_x(PERF + 0x58, 0);
                }
                m.set_f(PERF + 0x4c, vel);
                let g = accent_spread_gain(m);
                m.set_f(SB + 0x60 + 0x20, g * vel);
                smooth::start(m, SB, 12);
                restart_pattern(m);
            }
            _ => {}
        }
        m.set_u(GATE, 1);
        m.set_u(GATE + 0xc, 0);
        m.set_f(VEL, vel);
        m.set_b8(STARTED, 1);
        self.first = true;
    }

    /// `setNoteOff`: the Performer counts held notes; the last release stops shaking.
    pub fn note_off(&mut self) {
        let m = &mut self.m;
        let n = m.u(PERF);
        if n >= 2 {
            m.set_u(PERF, n - 1);
        } else {
            m.set_u(PERF + 0xc, 0);
            m.set_u(PERF, 0);
        }
    }

    pub fn choke(&mut self) {
        let m = &mut self.m;
        if m.u(GATE) == 1 {
            m.set_u(GATE, 2);
            m.set_f(GATE + 0xc, 1.0);
        }
        m.set_u(PERF + 0xc, 0);
    }

    /// `onBPMChanged` (`PercProcessor`'s inlined copy).
    pub fn set_tempo(&mut self, bpm: f64) {
        let m = &mut self.m;
        let bps = bpm * f64::from_bits(0x3f91_1111_1111_1111);
        m.set_d(PERF + 0x28, bps);
        let r = m.f(PERF + 0x40) as f64;
        let d = (r + r) * f64::from_bits(0x3f85_5555_5555_5555);
        m.set_d(PERF + 0x60, 1.0 / d);
        m.set_d(PERF + 0x50, bps / (d * m.f(PERF + 0x24) as f64));
    }

    /// `onPositionChanged(quarters, playing)`.
    pub fn position(&mut self, pos: f64, playing: bool) {
        let m = &mut self.m;
        if m.d(PERF + 0x30) != pos && m.b8(PERF + 0x10) != 0 {
            m.set_x(PERF + 0x58, 0);
        }
        let len = m.u(PERF + 0x44) as i32 as f64 * 12.0;
        m.set_d(PERF + 0x30, pos % len);
        if (m.b8(PERF + 0x38) != 0) != playing {
            restart_pattern(m);
        }
        m.set_b8(PERF + 0x38, playing as u8);
    }

    pub fn control(&mut self) {
        smooth::control(&mut self.m, SB, 12);
        smooth::control(&mut self.m, SA, 16);
    }

    /// Env Mode as set (+0xc8).
    pub fn env_mode(&self) -> u32 {
        self.m.u(MODE)
    }

    pub fn has_output(&self) -> bool {
        let m = &self.m;
        let active = match m.u(MODE) {
            0 => m.u(ENV_M + 0x10) != 0 || m.f(BLK_M + 0x10) > m.f(BLK_M + 0x20) || m.b8(BLK_M + 0x24) != 0,
            1 => m.u(ENV_R + 0x10) != 0 || m.f(BLK_R + 0x10) > m.f(BLK_R + 0x20) || m.b8(BLK_R + 0x24) != 0,
            _ => m.f(PERF + 0xc) > 0.0 || m.f(PERF + 0x68) > f32::from_bits(0x3a83_126f),
        };
        active && m.u(GATE) != 0
    }

    /// `processAudio` (0x10192be84): one block of at most 16 samples.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.has_output() {
            let ramp = smooth::active(&self.m, SA, 16) || smooth::active(&self.m, SB, 12);
            let first = self.first;
            for (a, b) in l.iter_mut().zip(r.iter_mut()) {
                let y = self.sample(ramp, first);
                *a = y;
                *b = y;
            }
            self.first = false;
            return;
        }
        if self.m.b8(STARTED) != 0 {
            self.reset();
        }
        l.fill(0.0);
        r.fill(0.0);
    }

    /// One sample of `processAudioImpl<mode, ramp, first>`. The first block after a
    /// reset recomputes every coefficient from its smoothed value; a ramp block only
    /// those whose value is moving.
    fn sample(&mut self, ramp: bool, first: bool) -> f32 {
        let m = &mut self.m;
        let upd = |m: &Mem, lane: usize| first || (ramp && m.f(SA + 0x40 + 4 * lane) != 0.0);
        let recompute = ramp || first;
        let t = exp_table();
        let mode = m.u(MODE_NOTE);
        if ramp {
            smooth::step(m, SA, 16);
        }
        let seed = m.u(NOISE + 0xc).wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        m.set_u(NOISE + 0xc, seed);
        let nx = m.f(NOISE + 4) * seed as i32 as f32;
        if upd(m, 12) {
            svf::set_freq(m, NSVF, t.hz(m.f(SA + 0x30)));
        }
        let lane12 = m.f(SA + 0x30);
        let nhp = svf::tick(m, NSVF, nx);
        let nbs = m.f(NSVF + 0xc);
        let sr = m.f(SR);
        let srk = clamp01((sr + f32::from_bits(0xc7ac_4400)) / f32::from_bits(0xc72c_4400));
        let gm = (lane12 + f32::from_bits(0xc296_0000)) / 45.0;
        let gm = if gm < 0.0 { 0.0 } else { gm };
        let gm = if gm > 1.0 { 0.3 } else { gm * f32::from_bits(0x3e99_999a) };
        // Grains.
        for i in 0..4 {
            let ph = m.f(PHASE + 4 * i);
            if ph >= 1.0 {
                let gr = GRN_R + 0x38 * i;
                let s = m.u(gr + 0xc).wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
                m.set_u(gr + 0xc, s);
                let x = m.f(gr + 4) * s as i32 as f32 * f32::from_bits(0x3ecc_cccd);
                m.set_f(RND + 4 * i, x);
                m.set_f(RATE + 4 * i, m.f(BASE + 4 * i) * (x + 1.0));
                let gl = GRN_L + 0x38 * i;
                let s = m.u(gl + 0xc).wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
                m.set_u(gl + 0xc, s);
                let y = m.f(gl + 4) * s as i32 as f32;
                let lvl = m.f(LEVEL + 4 * i);
                let g = GDEC + 0x14 * i;
                let new = if i == 0 {
                    let y = y + 1.0;
                    if y < lvl { lvl } else { y }
                } else {
                    let y = if y < lvl { lvl } else { y };
                    y + 1.0
                };
                if upd(m, 5 + i) {
                    let tm = m.f(SA + 0x14 + 4 * i);
                    if m.f(g) != tm {
                        m.set_f(g, tm);
                        m.set_f(g + 8, decay_coef(m.f(g + 0xc), tm));
                    }
                }
                m.set_f(g + 4, new);
                m.set_f(PHASE + 4 * i, 0.0);
            } else {
                m.set_f(PHASE + 4 * i, ph + m.f(RATE + 4 * i));
            }
        }
        let mut lv = [0f32; 4];
        for (i, l) in lv.iter_mut().enumerate() {
            let g = GDEC + 0x14 * i;
            let v = m.f(g + 8) * m.f(g + 4);
            m.set_f(g + 4, v);
            let fl = m.f(g + 0x10);
            *l = if v < fl {
                m.set_f(g + 4, 0.0);
                0.0
            } else {
                v - fl
            };
            m.set_f(LEVEL + 4 * i, *l);
        }
        if mode == 2 {
            return sample_performer(m, nhp, nbs, srk, gm, lv, recompute, ramp);
        }
        let mix = clamp01(m.f(SA + 0x28));
        if upd(m, 13) || (recompute && m.u(MODE) == 2) {
            svf::set_freq(m, SVF[0], t.hz(m.f(SA + 0x34)));
            svf::set_freq(m, SVF[1], t.hz(m.f(SA + 0x34) + 6.0));
        }
        if upd(m, 14) || (recompute && m.u(MODE) == 2) {
            svf::set_freq(m, SVF[2], t.hz(m.f(SA + 0x38)));
        }
        let x = nbs + gm * (nhp * srk);
        let x = x * ((((lv[0] + lv[1]) + lv[2]) + lv[3]) + -1.0).mul_add(mix, 1.0);
        let out = filters(m, x);
        let norm = follower(m, out);
        let (e, blk) = if mode == 0 { (ENV_M, BLK_M) } else { (ENV_R, BLK_R) };
        if mode == 0 {
            if upd(m, 2) {
                let rel = m.f(SA + 8);
                env::set_release(m, ENV_M, rel, LinShape);
                if m.f(BLK_M) != rel {
                    m.set_f(BLK_M, rel);
                    if m.b8(BLK_M + 4) != 0 {
                        block_coef(m, BLK_M, rel);
                    }
                }
            }
            if upd(m, 1) {
                let dec = m.f(SA + 4);
                env::set_decay(m, ENV_M, dec, LinShape);
            }
        } else if upd(m, 4) {
            let dec = m.f(SA + 0x10);
            env::set_decay(m, ENV_R, dec, InvLog);
            m.set_f(0x120, dec);
        }
        let vel = m.f(VEL);
        let kind = if mode == 0 { LinShape } else { InvLog };
        let ev = vel * env::process(m, e, kind);
        let lvl = block(m, blk, ev);
        let g = gate(m);
        let y = (out * f32::from_bits(0x3dcc_cccd)) / norm * f32::from_bits(0x3fc8_4c40);
        y * lvl * f32::from_bits(0x3f99_b363) * g
    }
}

impl Default for Shaker {
    fn default() -> Self {
        Shaker::new()
    }
}

/// Rate enum values (1/4 … 1/32) as steps per bar.
const RATES: [f32; 8] = [48.0, 32.0, 24.0, 16.0, 12.0, 8.0, 6.0, 1.0];

/// The 16-step shake shape of the Performer.
const SHAPE: [f32; 16] = [1.0, 0.84, 0.6, 0.333, 0.16, 0.04, 0.0, 0.02, 0.24, 0.42, 0.58, 0.71, 0.8, 0.87, 0.93, 0.96];

/// The three filters in series; returns the last high-pass output.
fn filters(m: &mut Mem, x: f32) -> f32 {
    svf::tick(m, SVF[0], x);
    let a = m.f(SVF[0] + 0x10);
    svf::tick(m, SVF[1], a);
    let b = m.f(SVF[1] + 0x10);
    svf::tick(m, SVF[2], b)
}

/// The envelope follower on |x|; returns max(follower, 0.001).
fn follower(m: &mut Mem, x: f32) -> f32 {
    let f = m.f(FOLLOW);
    let d = x.abs() - f;
    let c = if d > 0.0 { m.f(0x128) } else { m.f(0x12c) };
    let f = d.mul_add(c, f);
    m.set_f(FOLLOW, f);
    let lo = f32::from_bits(0x3a83_126f);
    if f < lo { lo } else { f }
}

/// The level block after the Machine/Realistic envelope: a decaying copy of the last
/// level crossfaded in, and a cubic fade-in over 15 ms once below the threshold.
fn block(m: &mut Mem, b: usize, ev: f32) -> f32 {
    let lv = m.f(b + 8);
    let held = lv * m.f(b + 0x14);
    m.set_f(b + 8, lv * m.f(b + 0xc));
    let mx = if held < ev { ev } else { held };
    let sum = ev + held;
    let k = clamp01(m.f(b + 0x18));
    let o = (sum - mx).mul_add(k, mx);
    let o = if o < 0.0 { 0.0 } else { o };
    m.set_f(b + 0x10, o);
    let mut f = 1.0;
    if o < m.f(b + 0x20) {
        let c = m.u(b + 0x30);
        let lim = m.u(b + 0x28);
        let c = if c < lim {
            c + 1
        } else {
            m.set_b8(b + 0x24, 0);
            lim
        };
        m.set_u(b + 0x30, c);
        let s = (-(c as f32)).mul_add(m.f(b + 0x2c), 1.0);
        f = s * (s * s);
    }
    o * f
}

fn gate(m: &mut Mem) -> f32 {
    match m.u(GATE) {
        0 => 0.0,
        1 => 1.0,
        _ => {
            let v = m.f(GATE + 0x10) * m.f(GATE + 0xc);
            m.set_f(GATE + 0xc, v);
            if (v as f64) < dsp::ENV_EPS {
                m.set_u(GATE + 0xc, 0);
                m.set_u(GATE, 0);
                0.0
            } else {
                v
            }
        }
    }
}

/// The decay factor of a level block from its time: `2^(−3/(sr·clamp(t)))`.
fn block_coef(m: &mut Mem, b: usize, t: f32) {
    let inv = m.f(b + 0x1c);
    let num = inv * -3.0;
    let cap = inv * f32::from_bits(0x4a00_2c80);
    let lo = f32::from_bits(0x38d1_b717);
    let t = if t < lo { lo } else { t };
    let d = if cap < t { cap } else { t };
    let e = exp2_s(num / d);
    let hi = f32::from_bits(0x3f7f_ff58);
    m.set_f(b + 0xc, if e > hi { hi } else { e });
}

/// Grain decay times (lanes 5–8) from Grain and the pitch.
fn grain_targets(m: &mut Mem, g: f32, p: f32) {
    let a = g * g.abs().mul_add(f32::from_bits(0xbf66_6666), f32::from_bits(0x3ff3_3333));
    let b = (p + f32::from_bits(0xc2d2_0000)) / -20.0;
    let a = if a < 0.0 { 0.0 } else { a };
    let b = if b < 0.0 { 0.0 } else { b };
    let a = if a > 1.0 { 1.0 } else { a };
    let b = if b > 1.0 { 1.0 } else { b };
    let x = f32::from_bits(0xbf4c_cccd).mul_add(a, f32::from_bits(0x3f99_999a));
    let y = f32::from_bits(0x3f4c_cccc).mul_add(b, 1.0);
    let k = x * y;
    for (i, c) in [0.005f32, 0.006, 0.007, 0.008].iter().enumerate() {
        m.set_f(SA + 0x80 + 0x14 + 4 * i, c * k);
    }
}

fn grain_lanes(m: &mut Mem, g: f32, p: f32) {
    grain_targets(m, g, p);
}

/// Filter pitches (lanes 13, 14) from Filter and the pitch.
fn filter_lanes(m: &mut Mem, f: f32, p: f32) {
    let c = clamp01(f);
    m.set_f(SA + 0x80 + 0x34, (142.0 - p).mul_add(c, p));
    m.set_f(SA + 0x80 + 0x38, p - c.mul_add(36.0, 0.0));
    smooth::start(m, SA, 16);
}

/// The one-pole high-pass set-up shared by the noise generators (+0 states, +0xc a, +0x10
/// b, +0x14 g, +0x18 f, +0x1c 1/sr).
fn one_pole(m: &mut Mem, b: usize, inv: f32, f: f32, pi_f: f32) {
    m.set_f(b + 0x1c, inv);
    let old = m.f(b + 0x18);
    if old == -1.0 {
        m.zero(b, 12);
        hp_coefs(m, b, inv, f, pi_f);
    } else {
        let g = dsp::tan_scalar(inv * (old * std::f32::consts::PI));
        m.set_f(b + 0x14, g);
        m.set_f(b + 0xc, 1.0 / (g + 1.0));
        m.set_f(b + 0x10, g + -1.0);
        m.zero(b + 4, 8);
        if old != f {
            hp_coefs(m, b, inv, f, pi_f);
        }
    }
    m.zero(b, 12);
}

fn hp_coefs(m: &mut Mem, b: usize, inv: f32, f: f32, pi_f: f32) {
    m.set_f(b + 0x18, f);
    let g = dsp::tan_scalar(inv * pi_f);
    m.set_f(b + 0x14, g);
    m.set_f(b + 0xc, 1.0 / (g + 1.0));
    m.set_f(b + 0x10, g + -1.0);
}

/// Pattern restart (phase and step state).
fn restart_pattern(m: &mut Mem) {
    m.set_u(PERF + 0x7c, 0);
    m.set_f(PERF + 0x84, m.f(PERF + 0x68));
}

/// Accent and Spread into the note's gain (dB, via the table).
fn accent_spread_gain(m: &Mem) -> f32 {
    let a = (m.f(PERF + 4) + f32::from_bits(0xbecc_cccd)) / f32::from_bits(0x3f19_999a);
    let b = (m.f(PERF + 8) + f32::from_bits(0xbecc_cccd)) / f32::from_bits(0x3f19_999a);
    let a = if a < 0.0 { 0.0 } else { a };
    let b = if b < 0.0 { 0.0 } else { b };
    let a = if a > 1.0 { -2.0 } else { a * -2.0 };
    let b = if b > 1.0 { 2.0 } else { b * 2.0 };
    exp_table().db(a + b)
}

/// `ShakerPerformer::setSampleRate`.
fn perf_set_sample_rate(m: &mut Mem, sr: f32, period: i32) {
    let p = PERF;
    m.set_f(p + 0x24, sr);
    let inv = 1.0 / sr;
    one_pole(m, p + 0x88, inv, 10.0, f32::from_bits(0x41fb_53d2));
    one_pole(m, p + 0xa8, inv, 50.0, f32::from_bits(0x431d_1463));
    m.set_f(p + 0xe0, sr);
    let (a, r) = (m.f(p + 0xc8), m.f(p + 0xcc));
    m.set_f(p + 0xc8, -1.0);
    m.set_f(p + 0xcc, -1.0);
    let ln2 = f32::from_bits(0x3f31_7215);
    if a != -1.0 {
        m.set_f(p + 0xc8, a);
        let x = a * sr;
        m.set_f(p + 0xd8, if x > 1.0 { ln2 / x } else { ln2 });
    }
    if r != -1.0 {
        m.set_f(p + 0xcc, r);
        let x = r * sr;
        m.set_f(p + 0xdc, if x > 1.0 { ln2 / x } else { ln2 });
    }
    m.set_u(p + 0xd0, 0);
    m.set_b8(p + 0xd4, 0);
    let st = ((sr * f32::from_bits(0x3d4c_cccd)) / period as f32 + 0.5) as i32;
    let st = if st > 1 { st } else { 1 };
    m.set_u(SB + 0x98, st as u32);
    m.set_f(SB + 0x9c, 1.0 / (st * period) as f32);
    m.set_f(p + 0x80, 1.0 / (sr * f32::from_bits(0x3c75_c28f)));
}

/// `ShakerPerformer::setAccent`.
fn perf_set_accent(m: &mut Mem, v: f32) {
    let p = PERF;
    m.set_f(p + 8, v);
    let c = clamp01(v);
    let a = c.mul_add(f32::from_bits(0x3cf5_c294), f32::from_bits(0x3df5_c28f));
    let sp = m.f(p + 0x18);
    let b = clamp01(v.mul_add(f32::from_bits(0x3ecc_cccd), sp) / f32::from_bits(0x3f33_3333));
    let b = b.mul_add(f32::from_bits(0x3f33_3333), f32::from_bits(0x3e99_999a));
    let a = a * b;
    let a = a * a;
    m.set_b8(SB + 0x90, 1);
    let s = clamp01(sp);
    let q = s.mul_add(-0.75, f32::from_bits(0x3f4c_cccd));
    let w = q.abs().mul_add(f32::from_bits(0x3f4c_cccd), f32::from_bits(0x3e4c_cccc));
    let w = q * w;
    let z = s.mul_add(f32::from_bits(0xbeb3_3334), f32::from_bits(0x3f19_999a)) - w;
    let w = z.mul_add(c, w);
    let s2 = s.mul_add(f32::from_bits(0xbe4c_cccd), f32::from_bits(0x3ecc_cccd));
    let s2 = s2.mul_add(c, 0.0);
    let c2 = c.mul_add(f32::from_bits(0x3eb3_3333), f32::from_bits(0x3ecc_cccd));
    m.set_f(SB + 0x60 + 4, a);
    m.set_f(SB + 0x60 + 8, w);
    m.set_f(SB + 0x60 + 0xc, s2);
    m.set_f(SB + 0x60 + 0x10, f32::from_bits(0x3dcc_cccd));
    m.set_f(SB + 0x60 + 0x14, v);
    m.set_f(SB + 0x60 + 0x18, c2);
    let g = accent_spread_gain(m);
    m.set_f(SB + 0x60 + 0x20, g * m.f(p + 0x4c));
    m.set_b8(SB + 0x90, 1);
    m.set_u(SB + 0xa0, 1);
}

/// `ShakerPerformer::setSpread`.
fn perf_set_spread(m: &mut Mem, v: f32) {
    let p = PERF;
    m.set_f(p + 4, v);
    let s = if m.b8(p + 0x11) != 0 || m.b8(p + 0x12) != 0 { v * f32::from_bits(0x3f19_999a) } else { v };
    m.set_f(p + 0x18, s);
    m.set_f(SB + 0x60, s);
    smooth::start(m, SB, 12);
    perf_set_accent(m, m.f(p + 8));
    let c = clamp01(m.f(p + 4));
    m.set_f(SB + 0x60 + 0x1c, c.mul_add(f32::from_bits(0x3cf5_c28f), f32::from_bits(0x3c23_d70a)));
    let g = accent_spread_gain(m);
    m.set_f(SB + 0x60 + 0x20, g * m.f(p + 0x4c));
    smooth::start(m, SB, 12);
}

/// The swing update shared by `setParamPfFill`.
fn swing(m: &mut Mem) {
    let (a, b, k) = if m.b8(PERF + 0x11) != 0 {
        (0.5, 0.5, f32::from_bits(0x3e4c_cccd))
    } else {
        let sw = m.f(PERF + 0x1c);
        let s = sw * sw.abs().mul_add(0.5, 0.5);
        let a = clamp01(s).mul_add(0.25, 0.5);
        let c = clamp01((s + f32::from_bits(0xbe4c_cccd)) / f32::from_bits(0x3f4c_cccd));
        let kk = f32::from_bits(0x3e19_999a);
        let b = a - m.f(PERF + 0x20) * c.mul_add(kk, kk);
        if m.b8(PERF + 0x12) != 0 { (0.5, 0.5, f32::from_bits(0x3e4c_cccd)) } else { (a, b, 1.0) }
    };
    m.set_f(SB + 0x60 + 0x24, a);
    m.set_f(SB + 0x60 + 0x28, b);
    smooth::start(m, SB, 12);
    m.set_f(PERF + 0x78, k);
}

fn set_double(m: &mut Mem, on: bool) {
    let changed = (m.b8(PERF + 0x11) != 0) != on;
    m.set_b8(PERF + 0x11, on as u8);
    if on {
        m.set_f(PERF + 0x70, 2.0);
    } else if m.b8(PERF + 0x12) == 0 {
        m.set_f(PERF + 0x70, 1.0);
    }
    perf_set_spread(m, m.f(PERF + 4));
    swing(m);
    if changed {
        restart_pattern(m);
    }
}

fn set_triple(m: &mut Mem, on: bool) {
    let changed = (m.b8(PERF + 0x12) != 0) != on;
    m.set_b8(PERF + 0x12, on as u8);
    if on {
        m.set_b8(PERF + 0x11, 0);
        m.set_f(PERF + 0x70, 1.5);
    } else if m.b8(PERF + 0x11) == 0 {
        m.set_f(PERF + 0x70, 1.0);
    }
    perf_set_spread(m, m.f(PERF + 4));
    swing(m);
    if changed {
        restart_pattern(m);
    }
}

/// `setParamPfFill(n)`: −1 Triple, 0 None, 1 Double.
fn pf_fill(m: &mut Mem, n: i32) {
    match n {
        -1 => {
            set_double(m, false);
            set_triple(m, true);
        }
        1 => {
            set_triple(m, false);
            set_double(m, true);
        }
        0 => {
            set_double(m, false);
            set_triple(m, false);
        }
        _ => {}
    }
}

/// `ShakerPerformer::processSampleImpl` (0x1019ba990; the ramp variant steps the 12
/// performer values first). Writes the pattern level (+0x6c) and the output (+0x68).
fn perf_sample(m: &mut Mem, ramp: bool) {
    let p = PERF;
    if ramp {
        smooth::step(m, SB, 12);
    }
    let mut d0 = m.d(p + 0x50) + m.d(p + 0x58);
    m.set_d(p + 0x58, d0);
    if m.b8(p + 0x10) != 0 {
        d0 = m.d(p + 0x30).mul_add(m.d(p + 0x60), d0);
    }
    let d0 = d0 - m.f(SB + 0x2c) as f64;
    let k = d0.mul_add(0.5, 1000.0) as i32;
    let s8 = {
        let v = k.wrapping_sub(0x3e8) as f32;
        v + v
    };
    let odd = (1 & !k) as f32;
    let sw = odd.mul_add(m.f(SB + 0x24), m.f(SB + 0x28) * (1.0 - odd));
    let ph = if sw != 0.0 && sw != 1.0 {
        let x = ((d0 - s8 as f64) as f32) * 0.5;
        let y = if x >= sw { (x - sw) / (1.0 - sw) + 1.0 } else { x / sw };
        y * 0.5
    } else {
        0.0
    };
    let pos = ph.mul_add(2.0, s8);
    let st = pos * m.f(p + 0x70) + -0.5 + 10.0;
    let st = (st - ((st + 0.5) as i32 as f32)) + 0.5;
    let shape = if m.u(p + 0x44) == 1 {
        f32::from_bits(0x3f4c_cccd)
    } else {
        let x = (pos as f64 - m.u(p + 0x14) as i32 as f64) as f32;
        let x = x.mul_add(m.f(p + 0x48), -0.5) + 10.0;
        let x = (x - ((x + 0.5) as i32 as f32)) + 0.5;
        let i = (x * 16.0) as i32;
        let x16 = x * 16.0;
        let fr = clamp01(x16 - x16 as i32 as f32);
        let a = SHAPE[(i & 0xf) as usize];
        let b = SHAPE[(i.wrapping_add(1) & 0xf) as usize];
        (b - a).mul_add(fr, a) + f32::from_bits(0xbe4c_cccd)
    };
    // The fill level through a one-pole low-pass (+0xa8).
    let y = m.f(p + 0xb4) * (-m.f(p + 0xb8)).mul_add(m.f(p + 0xa8), m.f(p + 0x78));
    let o = m.f(p + 0xbc) * (m.f(p + 0xa8) + y);
    m.set_f(p + 0xa8, y);
    m.set_f(p + 0xac, o);
    let g = clamp01(shape.mul_add(clamp01(o), f32::from_bits(0x3e4c_cccd)));
    let lvl = g.mul_add(f32::from_bits(0x3f7a_e148), f32::from_bits(0x3ca3_d70a));
    m.set_f(p + 0x6c, lvl);
    let (l1, l2, l3, l4, l5, l6) = (m.f(SB + 4), m.f(SB + 8), m.f(SB + 0xc), m.f(SB + 0x10), m.f(SB + 0x14), m.f(SB + 0x18));
    let d = lvl + -0.5;
    let d = d + d;
    let up = d.mul_add(l3, l2);
    let lim = f32::from_bits(0x3f7e_b852);
    let mut k1 = -(d.mul_add(l3, l2));
    if up > lim {
        k1 = -lim;
    }
    if k1 > lim {
        k1 = lim;
    }
    let k1 = k1 * (2.0 - k1.abs());
    let q2 = lvl.mul_add(l5, l4);
    let q2 = if q2 < -lim { -lim } else { q2 };
    let q2 = if q2 > lim { lim } else { q2 };
    let s5 = st + -0.5;
    let n = {
        let h = if s5 < 0.0 { -0.5 } else { 0.5 };
        (h + s5 as f64) as i32
    };
    let fr = s5 - n as f32;
    let half = l6 * 0.5;
    let e = if fr > half { 0.5 } else { -0.5 };
    let kc = if k1 < f32::from_bits(0xbf66_6666) { f32::from_bits(0xbf66_6666) } else { k1 };
    let x = if kc > f32::from_bits(0x38d1_b717) { f32::from_bits(0x3f80_0347) } else { kc + 1.0 };
    let q = 1.0 - x * x;
    // That through a one-pole low-pass (+0x88), input negated.
    let y = m.f(p + 0x94) * (-(m.f(p + 0x98).mul_add(m.f(p + 0x88), q)));
    let o2 = m.f(p + 0x9c) * (m.f(p + 0x88) + y);
    m.set_f(p + 0x88, y);
    m.set_f(p + 0x8c, o2);
    let q2c = clamp01(q2);
    let at = m.f(SB + 0x1c);
    let ln2 = f32::from_bits(0x3f31_7215);
    if m.f(p + 0xc8) != at {
        m.set_f(p + 0xc8, at);
        let x = at * m.f(p + 0xe0);
        m.set_f(p + 0xd8, if x > 1.0 { ln2 / x } else { ln2 });
    }
    if m.f(p + 0xcc) != at {
        m.set_f(p + 0xcc, at);
        let x = at * m.f(p + 0xe0);
        m.set_f(p + 0xdc, if x > 1.0 { ln2 / x } else { ln2 });
    }
    let fo = m.f(p + 0xd0);
    let dd = m.f(p + 0xc) - fo;
    let rising = dd > 0.0;
    m.set_b8(p + 0xd4, rising as u8);
    let c = if rising { m.f(p + 0xd8) } else { m.f(p + 0xdc) };
    let fo = fo + dd * c;
    m.set_f(p + 0xd0, fo);
    let out = if rising {
        let r = e / (e - half);
        let z = fr.mul_add(r, -(half * r));
        let z = (z + z).abs();
        let z = (z + z) + -1.0;
        let w = (z * (2.0 - z.abs())) * 0.5;
        let inv = 1.0 / o2;
        let w = w + 0.5;
        let b = inv + 1.0;
        let w = w + b;
        let w = o2 - o2 * w;
        let w = b + b / w;
        let v = (w - l1).mul_add(q2c, l1) * fo;
        m.set_f(p + 0x74, v);
        v
    } else {
        fo * m.f(p + 0x74)
    };
    let fade = clamp01(m.f(p + 0x80) + m.f(p + 0x7c));
    m.set_f(p + 0x7c, fade);
    let held = m.f(p + 0x84);
    m.set_f(p + 0x68, (out - held).mul_add(fade, held));
}

/// One sample of the Performer variant (`processAudioImpl<2, …>`).
#[allow(clippy::too_many_arguments)]
fn sample_performer(m: &mut Mem, nhp: f32, nbs: f32, srk: f32, gm: f32, lv: [f32; 4], recompute: bool, ramp: bool) -> f32 {
    let t = exp_table();
    let (s9, s11) = (m.f(SA + 0x34), m.f(SA + 0x38));
    perf_sample(m, ramp);
    let s7 = m.f(PERF + 0x6c);
    let s1 = m.f(PERF + 0x68);
    let s0 = m.f(SB + 0x20);
    let mix = clamp01((1.0 - s7) * m.f(SA + 0x2c));
    let f1 = s7.mul_add(5.0, s9) + -2.5;
    svf::set_freq(m, SVF[0], t.hz(f1));
    svf::set_freq(m, SVF[1], t.hz(f1 + 6.0));
    svf::set_freq(m, SVF[2], t.hz(s7.mul_add(-10.0, s11)));
    let _ = recompute;
    let x = nbs + gm * (nhp * srk);
    let e = (s1 + f32::from_bits(0xba83_126f)) * f32::from_bits(0x3f80_20cd);
    let x = x * ((((lv[0] + lv[1]) + lv[2]) + lv[3]) + -1.0).mul_add(mix, 1.0);
    let out = filters(m, x);
    let norm = follower(m, out);
    let y = (out * f32::from_bits(0x3dcc_cccd)) / norm * f32::from_bits(0x3fc8_4c40);
    let x = s0 * e * y;
    let st = m.f(HP12K);
    let h = m.f(HP12K + 0xc) * (-m.f(HP12K + 0x10)).mul_add(st, x);
    let hp = h - st;
    m.set_f(HP12K + 8, hp);
    m.set_f(HP12K, h);
    let g = gate(m);
    (x + e * (e + e) * hp) * f32::from_bits(0x3f99_b363) * g
}
