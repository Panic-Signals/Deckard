//! Percussion › Fractal: `NI::DSP::PercFBMatrixCore` (docs/synth-perc.md §4).
//!
//! Four sine-like oscillators (parabolic approximation) in a feedback matrix: each one's
//! pitch is bent by the other three's state-variable-filtered outputs ("FM"), and its
//! level is modulated by them ("AM"). Lane 0 (Freq A) is the output, mixed with a soft
//! clipped copy (Impact) and a band-passed noise burst, shaped by two `FreeEnvelope`s,
//! a pitch envelope, a declick crossfade and a 10 Hz high-pass. The four lanes are NEON
//! vectors in Maschine; here they are `[f32; 4]` with the same operation order.
//!
//! The state is a byte image of the C++ object (offsets below); every method follows
//! the assembly of the matching member function.

use super::dsp::env::Kind::LinLog;
use super::dsp::{
    self, LN8_NEG, Mem, V4, clamp01, env, exp_table, exp2_v, log2_v, smooth8, splat, tan_scalar, tan_v, vabs, vadd, vmax, vmin, vmul,
    vrecip, vsub,
};

/// Size of the object (it sits at +0xb80 of `PercProcessor`, the next core at +0x1280).
const SIZE: usize = 0x700;

// Parameters as last set (`setParameter` stores `v` at +0x80 + 4·idx).
const P_FM: usize = 0xa0; // idx 8
const P_AM: usize = 0xa4; // idx 9
const KEYTRACK: usize = 0xac; // u32: 0 Harmonic, 1 Dissonant
const TUNE_HOLD: usize = 0xb0; // u8
const TUNE: usize = 0xb4; // semitones −24..24
const FREQ_AB: usize = 0xb8; // Freq A, Freq B: 24..120 semitones
const FREQ_C: usize = 0xc0;
const DECAY: usize = 0xc4; // 15..45
const PITCH: usize = 0xc8; // 4 lanes: A, B, C, D = C + 4.5
const ENV2_DECAY: usize = 0xd8;
const ENV2_RELEASE: usize = 0xdc;
const ENV1_TIME: usize = 0xe0;
const AM: usize = 0xe4; // AM, FM as latched for the note
const FM: usize = 0xe8;
const GLIDED: usize = 0xf0; // 4 lanes
const ENV2_OUT: usize = 0x100;
const VEL_PEAK: usize = 0x104;
const NOISE_K: usize = 0x10c; // noise scale: 2^-31·sqrt(sr/44100)
const SEED: usize = 0x114;
const PHASE: usize = 0x140; // 4 lanes, and a copy at +0x150
const HZ: usize = 0x160;
const INC: usize = 0x180;
const INV_SR4: usize = 0x190;
const S1: usize = 0x1a0; // SVF band state
const S2: usize = 0x1b0; // SVF low state
const SVF_G: usize = 0x1f0;
const SVF_K: usize = 0x200;
const SVF_D: usize = 0x220;
const SR: usize = 0x230;
const INV_SR: usize = 0x234;
const OSC: usize = 0x240;
const FB: usize = 0x250; // sum of the other lanes' band outputs
const AM_GATE: usize = 0x290;
const PENV: usize = 0x2a0; // pitch envelope: time, level, factor, 1/sr, floor
const AENV: usize = 0x2b4; // a 2 ms decay (AM fade): time, level, factor, 1/sr, floor
const ENV1: usize = 0x2c8;
const ENV2: usize = 0x3c0;
const NSVF: usize = 0x4b8; // noise SVF: +0 band, +4 low, +8 hp, +0xc band sum, +0x10 low sum, +0x14 g, +0x18 k, +0x1c freq, +0x24 1/d, +0x28 sr, +0x2c 1/sr
const HPF: usize = 0x4e8; // 10 Hz one-pole high-pass: +0 state, +8 out, +0xc a, +0x10 b, +0x14 g, +0x18 freq, +0x1c 1/sr
const DECLICK: usize = 0x508;
const SHAPE: usize = 0x534;
const GATE: usize = 0x578; // u32 state, +0xc level, +0x10 factor, +0x14 1/sr
const SMOOTH: usize = 0x5a0; // SmoothingToolbox<8>: pitch A/B (impact), glide, impact amount, gain dB…
const VEL: usize = 0x620;
const NOTE: usize = 0x624;
const STARTED: usize = 0x628;
const CORE_SR: usize = 0x62c;
const INIT: usize = 0x634;

/// `PercFBMatrixCore`.
pub struct Fractal {
    m: Mem,
}

impl Fractal {
    /// The constructor (0x101921cbc).
    pub fn new() -> Fractal {
        let mut m = Mem::new(SIZE);
        let t = exp_table();
        m.set_u(KEYTRACK, 2);
        m.set_b8(TUNE_HOLD, 0);
        m.set_x(0x10c, 0x3f80_0000_3000_009d);
        m.set_f(0x11c, 1.0);
        m.set_f(0x138, 1.0);
        m.set_f(0x13c, 1.0);
        m.set_q(INV_SR4, splat(1.0));
        m.set_q(HZ, splat(1.0));
        m.set_q(SVF_K, splat(0.5));
        m.set_q(0x210, splat(1.0));
        m.set_q(SVF_D, splat(1.0));
        m.set_f(SR, 1.0);
        m.set_f(INV_SR, 1.0);
        m.set_f(PENV, 1.0);
        let half = clamp01(t.at(1140.0)); // value[1140] + slope·0
        m.set_f(0x2a8, half);
        m.set_f(0x2ac, 1.0);
        m.set_f(0x2b0, f32::from_bits(0x38d1_b717));
        m.set_f(AENV, 1.0);
        m.set_f(0x2b8, 0.0);
        m.set_f(0x2bc, half);
        m.set_f(0x2c0, 1.0);
        m.set_f(0x2c4, f32::from_bits(0x38d1_b717));
        for e in [ENV1, ENV2] {
            m.set_u(e, 0);
            m.set_d(e + 0x18, 1.0);
            for o in (0x58..=0x88).step_by(8) {
                m.set_d(e + o, 1.0);
            }
            m.set_f(e + 0xb0, 20000.0);
            m.set_f(e + 0xb4, 1.0);
            m.set_f(e + 0xb8, 1.0);
            m.set_f(e + 0xbc, 1.0);
            m.set_f(e + 0xd8, 1.0);
            m.set_f(e + 0xdc, 1.0);
            m.set_f(e + 0xec, -1.0);
            m.set_d(e + 0xf0, 1.0);
        }
        m.set_f(ENV2 + 0xa8, 0.5);
        m.set_f(NSVF + 0x28, 1.0);
        m.set_f(NSVF + 0x2c, 1.0);
        m.set_q(NSVF + 0x18, [0.5, 1.0, 0.75, 1.0]);
        m.set_f(DECLICK + 0x28, 1.0);
        m.set_f(DECLICK + 0x20, 0.5);
        m.set_f(DECLICK + 0x24, 0.5);
        m.set_q(HPF + 0x18, splat(1.0));
        m.set_f(GATE + 0x14, 1.0);
        m.set_q(GATE + 0x4, [1.0, 1.0, 0.0, f32::from_bits(0x3f00_0001)]);
        m.set_f(CORE_SR, -1.0);
        m.set_u(0x630, u32::MAX);
        // SVF coefficients for g = 0, k = 1.2.
        let k = splat(f32::from_bits(0x3f99_999a));
        let g = m.q(SVF_G);
        let d = vadd(vmul(g, vadd(g, k)), splat(1.0));
        smooth8::snap(&mut m, SMOOTH);
        m.set_q(SVF_K, k);
        m.set_q(SVF_D, vrecip(d));
        // Envelope 1: sustain 0.6; envelope 2: sustain 0.7; both without velocity.
        m.set_f(ENV1 + 0xa8, f32::from_bits(0x3f19_999a));
        m.set_d(ENV1 + 0x40, (-LN8_NEG / m.d(ENV1 + 0x68)).exp());
        m.set_d(ENV1 + 0xa0, -1.0);
        let r = (LN8_NEG / (m.d(ENV1 + 0x80) * m.f(ENV1 + 0xec) as f64)).exp();
        m.set_d(ENV1 + 0x48, r);
        m.set_f(ENV1 + 0xb4, 1.0);
        m.set_f(ENV1 + 0xb8, 0.0);
        m.set_f(ENV2 + 0xa8, f32::from_bits(0x3f33_3333));
        let sr2 = m.f(ENV2 + 0xec) as f64;
        m.set_d(ENV2 + 0x40, (LN8_NEG / (m.d(ENV2 + 0x68) * sr2)).exp());
        m.set_d(ENV2 + 0xa0, -1.0);
        m.set_d(ENV2 + 0x48, (LN8_NEG / (m.d(ENV2 + 0x80) * sr2)).exp());
        m.set_f(ENV2 + 0xb4, 1.0);
        m.set_f(ENV2 + 0xb8, 0.0);
        if m.f(NSVF + 0x20) != 0.0 {
            m.set_f(NSVF + 0x20, 0.0);
            m.set_f(NSVF + 0x18, 2.0);
            let g = m.f(NSVF + 0x14);
            m.set_f(NSVF + 0x24, 1.0 / g.mul_add(g + 2.0, 1.0));
        }
        // Waveshaper (positive: knee 0, limit 2·1.2, curve 0.8333, gain 1; negative:
        // knee 0, limit 2·0.8, curve 1.25, gain −1), clip ±2.4 with curve 0.8333.
        for (i, v) in [
            0x3f80_0000u32,
            0,
            0x3e4c_cccd,
            0,
            0x3f99_999a,
            0x3f99_999a,
            0x3f55_5555,
            0x3f80_0000,
            0,
            0x3f4c_cccd,
            0x3f4c_cccd,
            0x3fa0_0000,
            0xbf80_0000,
            0x3f99_999a,
            0x3f99_999a,
            0x4019_999a,
            0x3f55_5555,
        ]
        .iter()
        .enumerate()
        {
            m.set_u(SHAPE + 4 * i, *v);
        }
        Fractal { m }
    }

    /// `onSetSampleRate(sr, period)` (0x1019221ac).
    pub fn set_sample_rate(&mut self, sr: f32, period: i32) {
        let m = &mut self.m;
        let t = exp_table();
        smooth8::set_rate(m, SMOOTH, sr, period);
        m.set_f(CORE_SR, sr);
        m.set_u(0x630, period as u32);
        m.set_f(0x110, sr);
        let inv = 1.0 / sr;
        m.set_f(0x13c, inv);
        // A 15 Hz one-pole at +0x120 (coefficients only; unused by the audio code).
        let f0 = m.f(0x138);
        if f0 == -1.0 {
            m.set_x(0x120, 0);
            m.set_u(0x128, 0);
            hp_coefs(m, 0x120, inv, 15.0, f32::from_bits(0x423c_7edd));
        } else {
            let w = inv * (f0 * std::f32::consts::PI);
            let g = tan_poly(w);
            m.set_f(0x134, g);
            m.set_f(0x12c, 1.0 / (g + 1.0));
            m.set_f(0x130, g + -1.0);
            m.set_u(0x128, 0);
            m.set_u(0x124, 0);
            if f0 != 15.0 {
                hp_coefs(m, 0x120, inv, 15.0, f32::from_bits(0x423c_7edd));
            }
        }
        m.set_x(0x120, 0);
        m.set_u(0x128, 0);
        m.set_b8(0x108, 1);
        let s = (sr / 44100.0).sqrt();
        m.set_f(NOISE_K, s * f32::from_bits(0x3000_009d));
        m.set_f(0x11c, 1.0 / s);
        // Oscillators: increments, 1/sr, SVF coefficients for the current frequencies.
        let hz = m.q(HZ);
        let inc = vmax(vmin(vmul(hz, splat(inv)), splat(1.0)), splat(-1.0));
        m.set_q(INC, inc);
        m.set_q(INV_SR4, splat(inv));
        m.set_f(SR, sr);
        m.set_f(INV_SR, inv);
        let lim = sr * f32::from_bits(0x3efa_e148);
        m.zero(S1, 0x40);
        let inv12 = vrecip(splat(12.0));
        let pitch = m.q(0x260);
        let freq = vmul(pitch, inv12).map(|x| exp2_v(x + f32::from_bits(0x4042_01cc)));
        let freq = vmax(vmin(freq, splat(lim)), splat(0.0));
        let g = tan_v(vmul(vmul(freq, splat(std::f32::consts::PI)), splat(inv)));
        let k = m.q(SVF_K);
        let d = vadd(vmul(g, vadd(k, g)), splat(1.0));
        m.zero(0x1e0, 16);
        m.set_q(SVF_G, g);
        m.set_q(0x210, freq);
        m.set_q(SVF_D, vrecip(d));
        // Pitch envelope and AM fade: per-sample factors for their times.
        let ln = inv * f32::from_bits(0xbf31_7215);
        m.set_f(0x2ac, inv);
        m.set_f(0x2a4, 0.0);
        let f = (ln / (m.f(PENV) + f32::from_bits(0x0da2_4260))).mul_add(f32::from_bits(0x42ad_1f97), 1200.0);
        m.set_f(0x2a8, clamp01(t.at(f)));
        m.set_f(0x2c0, inv);
        m.set_u(0x2b8, 0);
        let a0 = m.f(AENV);
        let f = (ln / (a0 + f32::from_bits(0x0da2_4260))).mul_add(f32::from_bits(0x42ad_1f97), 1200.0);
        m.set_f(0x2bc, clamp01(t.at(f)));
        let ms2 = f32::from_bits(0x3b03_126f); // 0.002
        if a0 != ms2 {
            m.set_f(AENV, ms2);
            let f = (ln / ms2).mul_add(f32::from_bits(0x42ad_1f97), 1200.0);
            m.set_f(0x2bc, clamp01(t.at(f)));
        }
        env::set_rate(m, ENV1, sr, LinLog);
        if m.f(ENV1 + 0xec) > 0.0 {
            let att = f32::from_bits(0x3a03_126f); // 0.0005
            if m.f(ENV1 + 0xe0) != att {
                m.set_f(ENV1 + 0xe0, att);
                let lo = m.d(ENV1 + 0xf0) as f32;
                let s = dsp::fmax(lo, att); // fmaxnm
                let s = if s > 100.0 { 100.0 } else { s };
                m.set_d(ENV1 + 0x58, s as f64);
                m.set_d(ENV1 + 0x60, 1.0 / s as f64);
            }
        }
        env::set_rate(m, ENV2, sr, LinLog);
        // Noise band-pass.
        m.set_f(NSVF + 0x28, sr);
        m.set_f(NSVF + 0x2c, inv);
        let f1 = m.f(NSVF + 0x1c);
        m.set_f(NSVF + 0x1c, -1.0);
        if f1 != -1.0 {
            let lim = sr * f32::from_bits(0x3efa_e148);
            let f1 = if f1 < 0.0 { 0.0 } else { f1 };
            let f1 = if f1 > lim { lim } else { f1 };
            m.set_f(NSVF + 0x1c, f1);
            let g = tan_poly(inv * (f1 * std::f32::consts::PI));
            m.set_f(NSVF + 0x14, g);
            let k = m.f(NSVF + 0x18);
            m.set_f(NSVF + 0x24, 1.0 / g.mul_add(g + k, 1.0));
        }
        m.zero(NSVF, 0x14);
        // 10 Hz high-pass.
        m.set_f(HPF + 0x1c, inv);
        let fh = m.f(HPF + 0x18);
        if fh == -1.0 {
            m.set_d(HPF, 0.0);
            m.set_u(HPF + 8, 0);
            hp_coefs(m, HPF, inv, 10.0, f32::from_bits(0x41fb_53d2));
        } else {
            let g = tan_poly(inv * (fh * std::f32::consts::PI));
            m.set_f(HPF + 0x14, g);
            m.set_f(HPF + 0xc, 1.0 / (g + 1.0));
            m.set_f(HPF + 0x10, g + -1.0);
            m.set_d(HPF, 0.0);
            m.set_u(HPF + 8, 0);
            if fh != 10.0 {
                hp_coefs(m, HPF, inv, 10.0, f32::from_bits(0x41fb_53d2));
            }
        }
        // Declick ramp: 2/(rate·sr) per sample.
        m.set_f(DECLICK + 0x28, inv);
        let r = inv * m.f(DECLICK);
        m.set_f(DECLICK + 4, 1.0);
        m.set_x(DECLICK + 8, 0);
        m.set_x(DECLICK + 0x18, 0);
        m.set_x(DECLICK + 0x10, 0);
        m.set_f(DECLICK + 0x20, 2.0);
        m.set_f(DECLICK + 0x24, r + r);
        // Gate fade: 5 ms.
        m.set_f(GATE + 0x14, inv);
        m.set_u(GATE, 0);
        m.set_f(GATE + 8, f32::from_bits(0x3ba3_d70a));
        m.set_f(GATE + 0xc, 0.0);
        let e = (ln / f32::from_bits(0x3ba3_d70a)).exp();
        m.set_f(GATE + 0x10, if e > 1.0 { 1.0 } else { e });
        m.set_b8(INIT, 0);
    }

    /// `onReset` (0x1019229fc).
    pub fn reset(&mut self) {
        let m = &mut self.m;
        m.set_f(VEL, 0.0);
        m.set_b8(STARTED, 0);
        m.set_x(ENV2_OUT, 0);
        smooth8::snap(m, SMOOTH);
        m.zero(PHASE, 16);
        m.zero(S1, 0x50);
        m.set_u(0x2a4, 0);
        m.set_u(0x2b8, 0);
        m.set_x(ENV1 + 0x10, 0);
        m.set_u(ENV1, 0);
        m.set_x(ENV1 + 8, 0);
        m.set_d(ENV1 + 0x18, 1.0);
        m.set_d(ENV1 + 0x20, 0.0);
        m.set_x(ENV1 + 0xc0, 0);
        m.set_u(ENV1 + 0xc8, 0);
        m.set_u(ENV2, 0);
        m.zero(ENV2 + 8, 16);
        m.set_x(ENV2 + 0x20, 0);
        m.set_d(ENV2 + 0x18, 1.0);
        m.set_x(ENV2 + 0xc0, 0);
        m.set_u(ENV2 + 0xc8, 0);
        m.set_u(NSVF + 0x10, 0);
        m.zero(NSVF, 16);
        m.set_x(HPF, 0);
        m.set_u(HPF + 8, 0);
        m.set_f(DECLICK + 4, 1.0);
        m.set_x(DECLICK + 0x10, 0);
        m.set_x(DECLICK + 8, 0);
        m.set_x(DECLICK + 0x18, 0);
        m.set_u(GATE, 0);
        m.set_u(GATE + 0xc, 0);
    }

    /// `setParameter(idx, v)` (0x101922b70).
    pub fn set_param(&mut self, idx: u32, v: f32) {
        let m = &mut self.m;
        if idx > 10 {
            return;
        }
        m.set_f(0x80 + 4 * idx as usize, v);
        match idx {
            0 => m.set_b8(TUNE_HOLD, (v > 0.0) as u8),
            1 => m.set_u(KEYTRACK, v as u32),
            2 => m.set_f(TUNE, clamp01(v).mul_add(48.0, -24.0)),
            3 => m.set_f(DECAY, clamp01(v).mul_add(30.0, 15.0)),
            4 => {
                let v = (v.abs().mul_add(f32::from_bits(0xbf4c_cccd), f32::from_bits(0x3fe6_6666))) * v;
                let x = clamp01(v).mul_add(-82.0, 72.0);
                m.set_f(SMOOTH + 0x48, 1.0 / exp_table().hz(x));
                smooth8::start(m, SMOOTH);
            }
            5 => m.set_f(FREQ_AB, clamp01(v).mul_add(96.0, 24.0)),
            6 => m.set_f(FREQ_AB + 4, clamp01(v).mul_add(96.0, 24.0)),
            7 => m.set_f(FREQ_C, clamp01(v).mul_add(96.0, 24.0)),
            10 => {
                let a = clamp01((v + f32::from_bits(0xbe99_999a)) / f32::from_bits(0x3f33_3333));
                m.set_f(SMOOTH + 0x4c, a.mul_add(f32::from_bits(0x3f33_3333), 0.0));
                let c = clamp01(v);
                m.set_f(SMOOTH + 0x40, 18.0f32.mul_add(c, -12.0));
                m.set_f(SMOOTH + 0x44, (-12.0f32).mul_add(c, 12.0));
                m.set_f(SMOOTH + 0x50, clamp01(v / f32::from_bits(0x3f19_999a)));
                let c = clamp01(v / f32::from_bits(0x3e4c_cccd));
                m.set_f(SMOOTH + 0x54, c.mul_add(f32::from_bits(0xbc1b_a5e3), f32::from_bits(0x3c23_d70a)));
                smooth8::start(m, SMOOTH);
            }
            _ => {}
        }
    }

    /// `setNoteOn(velocity, note)` (0x101923034).
    pub fn note_on(&mut self, vel: f32, note: u32) {
        let m = &mut self.m;
        let t = exp_table();
        if !(vel >= 0.0 && vel <= 1.0) {
            m.set_f(VEL, 0.0);
            return;
        }
        smooth8::snap(m, SMOOTH);
        m.set_f(VEL, vel);
        let n = note as f32;
        m.set_f(NOTE, n);
        m.set_b8(STARTED, 1);
        if m.b8(TUNE_HOLD) != 0 {
            let base = n + -60.0 + m.f(TUNE);
            let harmonic = (m.u(KEYTRACK) as i32) < 1;
            let bc = [m.f(FREQ_AB + 4), m.f(FREQ_C)];
            // Lanes B, C: base + offset (harmonic) or offset + base·(0.73, 1.12).
            let lanes = if harmonic {
                [base + bc[0], base + bc[1]]
            } else {
                [f32::from_bits(0x3f3a_e148).mul_add(base, bc[0]), f32::from_bits(0x3f8f_5c29).mul_add(base, bc[1])]
            };
            m.set_f(PITCH + 4, lanes[0]);
            m.set_f(PITCH + 8, lanes[1]);
            let a = base + m.f(FREQ_AB);
            m.set_f(PITCH, a);
            let d = lanes[1] + 4.5;
            m.set_f(PITCH + 12, d);
            if m.b8(INIT) == 0 {
                m.set_q(GLIDED, [a, lanes[0], lanes[1], d]);
                m.set_b8(INIT, 1);
            }
            let y = pitch_decay(m, a);
            update_env_times(m, t, y);
            let am = m.f(P_AM);
            let fm = m.f(P_FM);
            m.set_f(AM, am);
            m.set_f(FM, fm);
        }
        env::note_on(m, ENV1, vel, LinLog);
        if m.f(ENV2 + 0xec) > 0.0 {
            let att = m.f(SMOOTH + 0x14);
            if m.f(ENV2 + 0xe0) != att {
                m.set_f(ENV2 + 0xe0, att);
                let lo = m.d(ENV2 + 0xf0) as f32;
                let s = if att < lo { lo } else { att };
                let s = if s > 100.0 { 100.0 } else { s };
                m.set_d(ENV2 + 0x58, s as f64);
                m.set_d(ENV2 + 0x60, 1.0 / s as f64);
            }
        }
        env::note_on(m, ENV2, vel, LinLog);
        m.set_f(0x2a4, 1.0);
        m.set_f(0x2b8, 1.0);
        let hz = t.hz(m.f(PITCH));
        let hz = dsp::fmax(hz, 1000.0); // fmaxnm
        m.set_f(DECLICK, hz);
        let r = m.f(DECLICK + 0x28) * hz * m.f(DECLICK + 0x20);
        m.set_f(DECLICK + 0x24, r);
        m.set_f(DECLICK + 4, 1.0);
        m.set_f(DECLICK + 8, 0.0);
        let held = m.f(DECLICK + 0x10) - m.f(DECLICK + 0x14);
        m.set_f(DECLICK + 0x18, held);
        m.set_f(DECLICK + 0x1c, m.f(DECLICK + 0xc));
        let p = m.f(ENV2_OUT);
        m.set_f(VEL_PEAK, if p > vel { p } else { vel });
        m.set_u(GATE, 1);
        m.set_u(GATE + 0xc, 0);
    }

    /// `onChoke`: fade out over the gate time.
    pub fn choke(&mut self) {
        if self.m.u(GATE) == 1 {
            self.m.set_u(GATE, 2);
            self.m.set_f(GATE + 0xc, 1.0);
        }
    }

    /// `processControlRate` (every 128 samples).
    pub fn control(&mut self) {
        smooth8::control(&mut self.m, SMOOTH);
    }

    pub fn has_output(&self) -> bool {
        (self.m.u(ENV2 + 0x10) != 0 || self.m.u(ENV1 + 0x10) != 0) && self.m.u(GATE) != 0
    }

    /// `processAudio` (0x101923990): one block of at most 16 samples, mono into both.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.has_output() {
            for (a, b) in l.iter_mut().zip(r.iter_mut()) {
                let y = self.sample();
                *a = y;
                *b = y;
            }
            return;
        }
        let m = &mut self.m;
        if m.b8(STARTED) != 0 {
            m.set_f(VEL, 0.0);
            m.set_b8(STARTED, 0);
            m.set_x(ENV2_OUT, 0);
            smooth8::snap(m, SMOOTH);
            m.zero(PHASE, 16);
            m.zero(S1, 0x50);
            m.set_u(0x2a4, 0);
            m.set_u(0x2b8, 0);
            m.set_x(ENV1 + 0x10, 0);
            m.set_u(ENV1, 0);
            m.set_x(ENV1 + 8, 0);
            m.set_x(ENV1 + 0x20, 0);
            m.set_d(ENV1 + 0x18, 1.0);
            m.set_x(ENV1 + 0xc0, 0);
            m.set_u(ENV1 + 0xc8, 0);
            m.set_u(ENV2, 0);
            m.set_x(ENV2 + 8, 0);
            m.set_x(ENV2 + 0x10, 0);
            m.set_x(ENV2 + 0x20, 0);
            m.set_d(ENV2 + 0x18, 1.0);
            m.set_x(ENV2 + 0xc0, 0);
            m.set_u(ENV2 + 0xc8, 0);
            m.set_u(NSVF + 0x10, 0);
            m.set_x(NSVF + 8, 0);
            m.set_x(NSVF, 0);
            m.set_x(HPF, 0);
            m.set_u(HPF + 8, 0);
            m.set_f(DECLICK + 4, 1.0);
            m.set_x(DECLICK + 0x10, 0);
            m.set_x(DECLICK + 8, 0);
            m.set_x(DECLICK + 0x18, 0);
            m.set_u(GATE, 0);
            m.set_u(GATE + 0xc, 0);
        }
        l.fill(0.0);
        r.fill(0.0);
    }

    /// One sample of `processAudioImpl<…>` (0x10199cc9c and its variants: glide on/off,
    /// Harmonic/Dissonant, parameter ramp active or not).
    fn sample(&mut self) -> f32 {
        let m = &mut self.m;
        let t = exp_table();
        if m.b8(SMOOTH + 0x61) != 0 {
            smooth8::step(m, SMOOTH);
        }
        let harmonic = m.u(KEYTRACK) == 0;
        let tgt: V4 = if harmonic {
            let base = m.f(NOTE) + -60.0 + m.f(TUNE);
            let c = base + m.f(FREQ_C);
            m.set_f(PITCH + 8, c);
            let a = base + m.f(FREQ_AB);
            let b = base + m.f(FREQ_AB + 4);
            m.set_f(PITCH, a);
            m.set_f(PITCH + 4, b);
            let d = c + 4.5;
            m.set_f(PITCH + 12, d);
            if m.b8(INIT) == 0 {
                m.set_q(GLIDED, [a, b, c, d]);
                m.set_b8(INIT, 1);
            }
            let y = pitch_decay(m, a);
            update_env_times(m, t, y);
            let (fm, am) = (m.f(P_FM), m.f(P_AM));
            m.set_f(AM, am);
            m.set_f(FM, fm);
            [a, b, c, d]
        } else {
            m.q(PITCH)
        };
        let fm = m.f(FM);
        let am = m.f(AM);
        // Glide (a one-pole on |pitch|, using last sample's value) or the pitch as is.
        let glide = m.f(SMOOTH + 8);
        let old = m.q(GLIDED);
        let cur = if glide > f32::from_bits(0x3b03_126f) {
            let g = glide * m.f(CORE_SR);
            let ln2 = f32::from_bits(0x3f31_7215);
            let c = if g < 1.0 { ln2 } else { ln2 / g };
            let d = vsub(vabs(tgt), old);
            m.set_q(GLIDED, vadd(old, vmul(d, splat(c))));
            old
        } else {
            m.set_q(GLIDED, tgt);
            tgt
        };
        // Pitch envelope: an exponential decay whose time follows the pitch.
        let x = (-0.5f32).mul_add(cur[0], 34.0);
        let a = t.db60(x);
        let fac = if m.f(PENV) != a {
            m.set_f(PENV, a);
            let s3 = m.f(0x2ac) * f32::from_bits(0xbf31_7215);
            let s2 = s3 / (a + f32::from_bits(0x0da2_4260));
            let f = clamp01(t.at(s2.mul_add(f32::from_bits(0x42ad_1f97), 1200.0)));
            m.set_f(0x2a8, f);
            f
        } else {
            m.f(0x2a8)
        };
        let pa = m.f(SMOOTH + 0x10);
        let lv = fac * m.f(0x2a4);
        m.set_f(0x2a4, lv);
        let fl = m.f(0x2b0);
        let lv = if lv < fl {
            m.set_u(0x2a4, 0);
            0.0
        } else {
            lv - fl
        };
        let pm = pa * 30.0 * lv;
        let pitch = vadd(cur, splat(pm));
        m.set_q(0x260, pitch);
        // Oscillator frequencies and SVF coefficients.
        let inv12 = vrecip(splat(12.0));
        let c3 = f32::from_bits(0x4042_01cc);
        let ratio: V4 = vmul(pitch, inv12).map(|x| exp2_v(x + c3));
        let lim = m.f(SR) * f32::from_bits(0x3efa_e148);
        let freq = vmax(vmin(ratio, splat(lim)), splat(0.0));
        let w = vmul(vmul(freq, splat(std::f32::consts::PI)), splat(m.f(INV_SR)));
        let g = tan_v(w);
        let k = m.q(SVF_K);
        let d = vadd(vmul(g, vadd(k, g)), splat(1.0));
        let rd = vrecip(d);
        m.set_q(SVF_G, g);
        m.set_q(0x210, freq);
        m.set_q(SVF_D, rd);
        // FM amount per lane (0.9, 0, 0.5, 0.5)·FM.
        let fmv = vmul([0.9, 0.0, 0.5, 0.5], splat(fm));
        m.set_q(0x280, fmv);
        // AM fade (2 ms).
        let lv2 = m.f(0x2bc) * m.f(0x2b8);
        m.set_f(0x2b8, lv2);
        let fl2 = m.f(0x2c4);
        let lv2 = if lv2 < fl2 {
            m.set_u(0x2b8, 0);
            0.0
        } else {
            lv2 - fl2
        };
        // Pitch bend from the other lanes: interpolate between log2(1/(1+fm)) and
        // log2(1+fm) by clamp(fb/4 + 0.5).
        let one = splat(1.0);
        let up = vadd(fmv, one);
        let down = vrecip(up);
        let ld = down.map(log2_v);
        let lu = up.map(log2_v);
        let fb = m.q(FB);
        let mix = vadd(vmul(fb, splat(0.25)), splat(0.5));
        let ld = vmul(vadd(ld, splat(-c3)), splat(12.0));
        let lu = vmul(vadd(lu, splat(-c3)), splat(12.0));
        let mix = vmax(vmin(mix, one), splat(0.0));
        let bend = vadd(ld, vmul(mix, vsub(lu, ld)));
        let bend: V4 = vmul(inv12, bend).map(|x| exp2_v(x + c3));
        let fa = clamp01(lv2.mul_add(-4.0, f32::from_bits(0x3f99_999a)));
        let amv = vmul([0.8, 0.0, 0.8, 0.9], splat(am * fa));
        m.set_q(0x270, amv);
        // The unclamped frequency times the bend.
        let hz = vmul(ratio, bend);
        let inc = vmax(vmin(vmul(m.q(INV_SR4), hz), one), splat(-1.0));
        m.set_q(HZ, hz);
        m.set_q(INC, inc);
        let p = vadd(vadd(inc, m.q(PHASE)), splat(10.0));
        let r = vadd(p, splat(0.5)).map(|x| if x.abs() < f32::from_bits(0x4b80_0000) { (x as i32) as f32 } else { x });
        let ph = vsub(p, r);
        m.set_q(PHASE, ph);
        m.set_q(PHASE + 16, ph);
        let tri = vsub(p, r).map(f32::abs);
        let tri = vadd(vmul(tri, splat(-16.0)), splat(8.0));
        let sine = vmul(ph, tri);
        let fbam = vmul(fb, amv);
        let amc = vmax(vmin(amv, one), splat(0.0));
        let amd = vmul(vadd(fb, splat(-1.0)), amc);
        let gate = m.q(AM_GATE);
        let amm: V4 = std::array::from_fn(|i| if gate[i] > 0.0 { fbam[i] } else { amd[i] });
        let x = vmul(vadd(amm, one), sine);
        // TPT state-variable filter per lane.
        let s1 = m.q(S1);
        let s2 = m.q(S2);
        let g2 = vmul(g, g);
        let s1x2 = vadd(s1, s1);
        let acc = vadd(vadd(s2, s2), vmul(g, s1x2));
        let g2 = vadd(g2, vmul(g, k));
        let acc = vadd(acc, vmul(s1x2, k));
        let acc = vadd(acc, vmul(x, g2));
        let hp = vsub(x, vmul(rd, acc));
        let bp = vadd(s1, vmul(g, hp));
        let bsum = vadd(s1, bp);
        m.set_q(0x1c0, hp);
        m.set_q(0x1d0, bsum);
        let lp = vadd(s2, vmul(g, bsum));
        m.set_q(0x1e0, vadd(s2, lp));
        m.set_q(S1, bp);
        m.set_q(S2, lp);
        let tot = (bsum[0] + bsum[1]) + (bsum[2] + bsum[3]);
        m.set_q(OSC, x);
        m.set_q(FB, vsub(splat(tot), bsum));
        // Impact: soft clip of the first oscillator mixed in.
        let lim = m.f(SHAPE + 0x3c);
        let s = 0.5 * x[0];
        let s = if s < -lim { -lim } else { s };
        let s12 = if s > lim { lim } else { s };
        let s15 = m.f(SHAPE + 0x40);
        let s13 = clamp01(m.f(SMOOTH + 0xc));
        // Noise: LCG through a band-pass at the pitch + 16 semitones.
        let seed = m.u(SEED).wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        m.set_u(SEED, seed);
        let nf = t.hz(cur[0] + 16.0);
        let (ng, nk, nd) = if m.f(NSVF + 0x1c) != nf {
            let lim = m.f(NSVF + 0x28) * f32::from_bits(0x3efa_e148);
            let f = if nf < 0.0 { 0.0 } else { nf };
            let f = if f > lim { lim } else { f };
            m.set_f(NSVF + 0x1c, f);
            let g = tan_scalar(m.f(NSVF + 0x2c) * (f * std::f32::consts::PI));
            m.set_f(NSVF + 0x14, g);
            let k = m.f(NSVF + 0x18);
            let d = 1.0 / g.mul_add(k + g, 1.0);
            m.set_f(NSVF + 0x24, d);
            (g, k, d)
        } else {
            (m.f(NSVF + 0x14), m.f(NSVF + 0x18), m.f(NSVF + 0x24))
        };
        let nx = m.f(NOISE_K) * seed as i32 as f32;
        let b0 = m.f(NSVF);
        let l0 = m.f(NSVF + 4);
        let b2 = b0 + b0;
        let a = l0.mul_add(2.0, ng * b2);
        let gg = ng.mul_add(nk, ng * ng);
        let a = b2.mul_add(nk, a);
        let a = nx.mul_add(gg, a);
        let nhp = nx - nd * a;
        m.set_f(NSVF + 8, nhp);
        let nb = ng.mul_add(nhp, b0);
        let nbs = b0 + nb;
        m.set_f(NSVF + 0xc, nbs);
        let nl = ng.mul_add(nbs, l0);
        m.set_f(NSVF + 0x10, l0 + nl);
        m.set_f(NSVF, nb);
        m.set_f(NSVF + 4, nl);
        // Envelope 1 (noise level), gain, Impact mix and waveshaper.
        let vel = m.f(VEL);
        let e1 = env::process(m, ENV1, LinLog);
        let gain = t.db(m.f(SMOOTH));
        let b0v = bsum[0];
        let sc = (s12.abs() * -0.25).mul_add(s15, 1.0);
        let sc = s12 * sc - b0v;
        let mix = sc.mul_add(s13, b0v);
        let nz = vel * e1 * f32::from_bits(0x3e19_999a);
        let v = (mix + nbs * nz) * gain;
        let shaped = shape(m, v);
        let g2 = t.db(m.f(SMOOTH + 4));
        let (d2, r2) = (m.f(ENV2_DECAY), m.f(ENV2_RELEASE));
        env::set_decay(m, ENV2, d2, LinLog);
        env::set_release(m, ENV2, r2, LinLog);
        let pre = shaped * g2;
        let e2 = env::process(m, ENV2, LinLog) * m.f(VEL_PEAK);
        m.set_f(ENV2_OUT, e2);
        let mut o = pre * e2;
        // Declick: crossfade from the previous note's last output.
        m.set_f(DECLICK + 0x14, m.f(DECLICK + 0x10));
        m.set_f(DECLICK + 0xc, o);
        let ph = m.f(DECLICK + 4);
        if ph > 0.0 {
            let ph = ph - m.f(DECLICK + 0x24);
            m.set_f(DECLICK + 4, ph);
            let h = ph + -0.5;
            let h = (-h).mul_add(h.abs(), h);
            let w = h.mul_add(2.0, 0.5);
            let base = m.f(DECLICK + 8);
            let x = (m.f(DECLICK + 0x1c) + base) * w;
            m.set_f(DECLICK + 8, base + m.f(DECLICK + 0x18));
            o += x;
        }
        m.set_f(DECLICK + 0x10, o);
        // 10 Hz high-pass.
        let st = m.f(HPF);
        let y = m.f(HPF + 0xc) * (-m.f(HPF + 0x10)).mul_add(st, o);
        let out = y - st;
        m.set_f(HPF + 8, out);
        m.set_f(HPF, y);
        // Gate.
        let gv = match m.u(GATE) {
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
        };
        out * f32::from_bits(0x3eef_340d) * gv
    }
}

impl Default for Fractal {
    fn default() -> Self {
        Fractal::new()
    }
}

/// Decay of envelope 1 from the pitch: `y = (D+20) + (D − (D+20))·clamp((A − 24)/96)`.
fn pitch_decay(m: &Mem, a: f32) -> f32 {
    let x = clamp01((a + -24.0) / 96.0);
    let dcy = m.f(DECAY);
    let hi = dcy + 20.0;
    (dcy - hi).mul_add(x, hi)
}

/// Envelope times from `y`: env 2 release `y`, env 2 decay `min(y − 15, 24)`, env 1
/// `clamp(y − 16, 17, 26)` (all as dB-below-60 values → seconds).
fn update_env_times(m: &mut Mem, t: &dsp::ExpTable, y: f32) {
    m.set_f(ENV2_RELEASE, t.db60(y));
    let a = y + -15.0;
    let a = if a > 24.0 { 24.0 } else { a };
    m.set_f(ENV2_DECAY, t.db60(a));
    let b = y + -16.0;
    let b = if b < 17.0 { 17.0 } else { b };
    let b = if b > 26.0 { 26.0 } else { b };
    let e = t.db60(b);
    m.set_f(ENV1_TIME, e);
    env::set_decay(m, ENV1, e, LinLog);
    env::set_release(m, ENV1, e, LinLog);
}

/// The waveshaper (+0x534): linear up to a knee, then `x·(1 − c·x/4)` up to twice the
/// limit; separate positive and negative halves.
fn shape(m: &Mem, v: f32) -> f32 {
    let (knee, lim, c, g) = if v > 0.0 {
        (m.f(SHAPE + 0xc), m.f(SHAPE + 0x14), m.f(SHAPE + 0x18), m.f(SHAPE + 0x1c))
    } else {
        (m.f(SHAPE + 0x20), m.f(SHAPE + 0x28), m.f(SHAPE + 0x2c), m.f(SHAPE + 0x30))
    };
    let x = if v > 0.0 { v } else { -v };
    if knee > x {
        return if v > 0.0 { x * g } else { g * x };
    }
    let d = x - knee;
    let l2 = lim + lim;
    let d = if l2 < d { l2 } else { d };
    let p = (d * -0.25).mul_add(c, 1.0);
    g * d.mul_add(p, knee)
}

/// `tan` approximation with the scalar code's constants.
fn tan_poly(w: f32) -> f32 {
    tan_scalar(w)
}

/// One-pole high-pass coefficients at `f` Hz, `pi_f` = f·π as a constant (writes +0x14
/// g, +0xc 1/(g+1), +0x10 g−1, +0x18 f).
fn hp_coefs(m: &mut Mem, b: usize, inv: f32, f: f32, pi_f: f32) {
    m.set_f(b + 0x18, f);
    let w = inv * pi_f;
    let g = tan_poly(w);
    m.set_f(b + 0x14, g);
    m.set_f(b + 0xc, 1.0 / (g + 1.0));
    m.set_f(b + 0x10, g + -1.0);
}
