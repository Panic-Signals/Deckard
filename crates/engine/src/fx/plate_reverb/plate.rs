//! `NI::DSP::PlateReverb`, the Plate mode of Maschine's Reverb (docs/fx-plate-reverb.md §4).
//!
//! A hand-written 4-lane SIMD reverb (NI's SSE emulation on NEON): an input chain of 20
//! one-sample allpasses and a pipeline of four first-order filters, 32 early taps, a
//! 16-line feedback delay network with a Hadamard mix and a biquad damping filter per line,
//! a stereo "width" matrix and an output pre-delay. Field names follow the struct offsets
//! in the binary (`_0x..` comments). Every vector operation is done lane by lane with the
//! same rounding; scalar code uses `mul_add` exactly where Maschine has `fmadd`.

#![allow(clippy::needless_range_loop)] // lane-indexed loops mirror the binary's SIMD lanes
use super::{c, cos_poly, exp10f, sin_poly};

/// Lines of the delay network and their buffer length (indices wrap after 21199).
const LINES: usize = 16;
const LINE_LEN: usize = 21200;
/// Early-reflection buffer (indices wrap after 23999).
const EARLY_LEN: usize = 24000;
/// Output pre-delay buffer per channel (`std::vector` of 96100; indices wrap after 96099).
const PD_LEN: usize = 96100;
const PD_WRAP: i32 = 96099;

const PI: f32 = c(0x4049_0fdb);
const QUARTER_PI: f32 = c(0x3f49_0fdb);
const HALF_PI: f32 = c(0x3fc9_0fdb);
/// −ln 1000 (`0xc0dd0c55`): a T60 decay exponent.
const NEG_LN_1000: f32 = c(0xc0dd_0c55);
const LOG2_E: f32 = c(0x3fb8_aa11);
/// Coefficients of `powe_0p02prec`'s 2^f polynomial.
const P2: [f32; 4] = [c(0x3d65_6042), c(0x3e77_ced9), c(0x3f31_6873), 1.0];
/// Early-tap delays, seconds (×sr, truncated).
const EARLY_SECS: [u32; 32] = [
    0x3cff513d, 0x3cee147b, 0x3cf4e81b, 0x3d119423, 0x3d3f3b64, 0x3b656042, 0x3c289e61, 0x3c70fb39, 0x3aaec33e, 0x3c2b596e, 0x3c555555,
    0x3983126f, 0x3bfd44f3, 0x37aec33e, 0x3b9374bc, 0x3cf5ee40, 0x3ce8ca12, 0x3d4ead66, 0x3c919423, 0x3cd08dff, 0x3cfced91, 0x3c9c8057,
    0x3c369d03, 0x3c5a1cac, 0x3d0e6bdd, 0x3d131d5b, 0x3d7f6715, 0x3c3dc805, 0x3cc131d6, 0x3d33e1f6, 0x3aa11bfd, 0x3d028f5c,
];
/// Line lengths, seconds.
const LINE_SECS: [u32; 16] = [
    0x3cad3a07, 0x3caf4651, 0x3cc15d86, 0x3ccccccd, 0x3cd81062, 0x3cee147b, 0x3cfd1942, 0x3d069217, 0x3d0d3a07, 0x3d1b0cf8, 0x3d2402bb,
    0x3d2dd2f2, 0x3d3a1cac, 0x3d460f05, 0x3d50cf88, 0x3d618937,
];
/// Signs with which the early taps join each line.
const INJECT: [f32; 16] = [1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0];
/// Weights of the second early-tap set in the direct early sum.
const TAP_W: [u32; 32] = [
    0xbf839f56, 0x3f867d56, 0xbf8559b4, 0x3f7bb98c, 0xbf60a3d7, 0xbfaea162, 0xbfa2ee63, 0xbf9bb98c, 0x3fb28588, 0x3fa2a305, 0x3f9e703b,
    0xbfb4816f, 0x3fa73eab, 0x3fb4ed91, 0xbfacd9e8, 0x3f852bd4, 0xbf875c29, 0xbf58240b, 0xbf96ecc0, 0xbf8b8866, 0xbf8401a3, 0xbf94e076,
    0xbfa17f63, 0xbf9dfa44, 0x3f7db8bb, 0x3f7ac711, 0xbf3f6944, 0xbfa0c49c, 0xbf8e353f, 0x3f6710cb, 0xbfb2b6ae, 0xbf82a993,
];
/// The early taps' gain (`0x3f642905`, −1 dB).
const EARLY_GAIN: f32 = c(0x3f64_2905);

/// `powe_0p02prec`: 2^z by rounding to the nearest integer and a cubic for the fraction.
fn pow2_approx(z: f32) -> f32 {
    let n = z.round_ties_even();
    let ni = if n > -2147483648.0 && n < 2147483648.0 { n as i32 } else { i32::MIN };
    let f = z - ni as f32;
    let e = f32::from_bits((ni.wrapping_add(127) as u32) << 23);
    let p = P2[3] + f * (P2[2] + f * (P2[1] + f * P2[0]));
    p * e
}

/// One first-order shelf design (`compute*FilterCoeffs`, §4.5): gain `g` (a power ratio
/// pair `k`, `g`), corner `f` Hz. Returns (gain, a, b) of `y = g·(x + a·x₋₁) − b·y₋₁`.
fn shelf(g: f32, k: f32, k2: f32, f: f32, sr: f32, ab: impl Fn(f32, f32) -> (f32, f32), qn: f32) -> (f32, f32, f32) {
    let s = (g * k).sqrt();
    let d = (s * s).mul_add(-2.0, g * g + k2);
    let mut alpha = 0.0;
    if d != 0.0 {
        let q = qn / d;
        let sg = if q < 0.0 { -1.0 } else { 1.0 };
        alpha = q - sg * q.mul_add(q, -1.0).sqrt();
    }
    let r = f / sr;
    let w = if r < 1.0 { r * PI } else { PI };
    let (a, b) = ab(g, alpha);
    let a = a * 0.5;
    let b = b * 0.5;
    let t = w.mul_add(0.5, -QUARTER_PI).sin() / w.mul_add(0.5, QUARTER_PI).sin();
    let n = 1.0 / t.mul_add(alpha, 1.0);
    let gain = n * t.mul_add(b, a);
    let x = n * t.mul_add(a, b);
    let bb = (t + alpha) * n;
    (gain, x / gain, bb)
}

pub(crate) struct Plate {
    /// `+0x8`: parameters jump instead of ramping (before the first resume).
    immediate: bool,
    // Stored parameters (`+0x18..0x2c`).
    p_predelay: f32,
    p_decay: f32,
    p_mix: f32,
    p_hidamp: f32,
    p_lowshelf: f32,
    p_width: f32,
    sr: f32,
    inv_sr: f32,
    wet_gain: f32,
    /// `+0xc4`: T60s (ms) of the mid, low and high bands, and the low crossover (Hz).
    t60: [f32; 4],
    /// `+0xd4`: high crossover (Hz).
    f_hi: f32,
    // Damping biquads, one per line (`+0xe0..0x2f0`).
    b0: [f32; LINES],
    b1: [f32; LINES],
    b2: [f32; LINES],
    a1: [f32; LINES],
    a2: [f32; LINES],
    x1: [f32; LINES],
    x2: [f32; LINES],
    y1: [f32; LINES],
    y2: [f32; LINES],
    // Input filter pipeline (`+0x330..0x370`): packed coefficients, states, gain.
    pa: [f32; 4],
    pb: [f32; 4],
    px: [f32; 4],
    py: [f32; 4],
    pgain: f32,
    // The four filters' own coefficients (`+0x374..0x3c4`).
    hs1_g: f32,
    hs1_k: f32,
    hs1_a: f32,
    hs1_b: f32,
    ls_g: f32,
    ls_k: f32,
    ls_a: f32,
    ls_b: f32,
    lp_k: f32,
    lp_a: f32,
    lp_b: f32,
    hs2_g: f32,
    hs2_k: f32,
    hs2_a: f32,
    hs2_b: f32,
    /// `+0x3d0`: the input allpass chain.
    ap: [f32; 20],
    /// `+0x420`: the lines' outputs.
    out: [f32; LINES],
    /// `+0x4f0`: line lengths in samples.
    len: [f32; LINES],
    w: [i32; LINES],
    r: [i32; LINES],
    lines: Vec<f32>,
    early: Vec<f32>,
    ew: i32,
    tap1: [i32; 32],
    tap2: [i32; 32],
    d_early: [i32; 32],
    pd_w: i32,
    pd_r: i32,
    pd: [Vec<f32>; 2],
    /// `+0x1633e0`: the width matrix rows.
    sinv: [f32; 4],
    cosv: [f32; 4],
    // Parameter ramps (`+0x163410..0x163480`).
    cur: [f32; 8],
    inc: [f32; 8],
    tgt: [f32; 8],
    dirty: bool,
    ramping: bool,
    countdown: i32,
    ticks: i32,
    kinc: f32,
}

impl Plate {
    /// The constructor, `onInit()` and `onSetSampleRate(44100, 128)`, as
    /// `PlateReverbProcessor`'s constructor runs them.
    pub(crate) fn new() -> Box<Plate> {
        let mut p = Box::new(Plate {
            immediate: true,
            p_predelay: 0.0,
            p_decay: 0.5,
            p_mix: 0.1,
            p_hidamp: 0.5,
            p_lowshelf: 0.5,
            p_width: 0.0,
            sr: 0.0,
            inv_sr: 0.0,
            wet_gain: 1.0,
            t60: [0.0, 0.0, 0.0, 1000.0],
            f_hi: 10000.0,
            b0: [0.0; LINES],
            b1: [0.0; LINES],
            b2: [0.0; LINES],
            a1: [0.0; LINES],
            a2: [0.0; LINES],
            x1: [0.0; LINES],
            x2: [0.0; LINES],
            y1: [0.0; LINES],
            y2: [0.0; LINES],
            pa: [0.0; 4],
            pb: [0.0; 4],
            px: [0.0; 4],
            py: [0.0; 4],
            pgain: 0.0,
            hs1_g: 1.0,
            hs1_k: 1.0,
            hs1_a: 0.0,
            hs1_b: 0.0,
            ls_g: 1.0,
            ls_k: 1.0,
            ls_a: 0.0,
            ls_b: 0.0,
            lp_k: 1.0,
            lp_a: 0.0,
            lp_b: 0.0,
            hs2_g: 1.0,
            hs2_k: 1.0,
            hs2_a: 0.0,
            hs2_b: 0.0,
            ap: [0.0; 20],
            out: [0.0; LINES],
            len: [0.0; LINES],
            w: [0; LINES],
            r: [0; LINES],
            lines: vec![0.0; LINES * LINE_LEN],
            early: vec![0.0; EARLY_LEN],
            ew: 0,
            tap1: [0; 32],
            tap2: [0; 32],
            d_early: [0; 32],
            pd_w: 0,
            pd_r: 0,
            pd: [vec![0.0; PD_LEN], vec![0.0; PD_LEN]],
            sinv: [0.0; 4],
            cosv: [0.0; 4],
            cur: [0.0; 8],
            inc: [0.0; 8],
            tgt: [0.0; 8],
            dirty: false,
            ramping: false,
            countdown: 0,
            ticks: 0,
            kinc: 0.0,
        });
        p.on_init();
        p.set_sample_rate(44100.0, 128);
        p
    }

    fn on_init(&mut self) {
        self.cur = [0.0; 8];
        self.inc = [0.0; 8];
        self.tgt = [0.0; 8];
        self.dirty = false;
        self.ramping = false;
        self.countdown = 0;
        self.inv_sr = 1.0;
        self.p_predelay = 0.0;
        self.p_decay = 0.5;
        self.tgt[1] = 0.5;
        self.dirty = true;
        self.t60 = [3756.0, 708.0, 2503.5, 1346.5];
        self.hs1_g = exp10f(c(0x3d99_999a));
        self.tgt[2] = 0.1;
        self.p_mix = 0.1;
        self.p_hidamp = 0.5;
        self.f_hi = 8425.5;
        self.hs2_g = exp10f(c(0xbdcc_cccd));
        self.p_lowshelf = 0.5;
        self.tgt[4] = 0.5;
        self.tgt[3] = 0.5;
        self.ls_g = exp10f(c(0xbf14_0000));
        self.p_width = 1.0;
        self.tgt[5] = 1.0;
        // The width matrix for width 1: sin and cos of [0, 0, π/2, π/2].
        let x = [0.0, 0.0, HALF_PI, HALF_PI];
        for i in 0..4 {
            let x2 = [0.0, 0.0, c(0x401d_e9e7), c(0x401d_e9e7)][i];
            self.sinv[i] = sin_poly(x2) * x[i];
            self.cosv[i] = cos_poly(x2);
        }
        self.set_parameter(7, self.p_mix);
        self.reset();
        self.cur = self.tgt;
        self.dirty = true;
    }

    /// `onSetSampleRate(sr, block)`.
    pub(crate) fn set_sample_rate(&mut self, sr: f32, block: i32) {
        let n = ((sr * c(0x3ca3_d70a)) / block as f32 + 0.5) as i32;
        let n = if n > 1 { n } else { 1 };
        self.ticks = n;
        self.kinc = 1.0 / (n * block) as f32;
        self.sr = sr;
        self.inv_sr = 1.0 / sr;
        self.sample_rate_things();
    }

    fn sample_rate_things(&mut self) {
        // computeDelaysInSamples
        for (d, s) in self.d_early.iter_mut().zip(EARLY_SECS) {
            *d = (c(s) * self.sr) as i32;
        }
        for (l, s) in self.len.iter_mut().zip(LINE_SECS) {
            *l = c(s) * self.sr;
        }
        self.pd_w = PD_WRAP;
        // A first-order lowpass at 7.5 kHz (`0x473813e4` = 2π·7500).
        let t = (self.inv_sr * c(0x4738_13e4) * 0.5).tan();
        let i = 1.0 / t;
        let s = i + 1.0;
        self.lp_k = 1.0 / s;
        self.lp_b = -(i + -1.0) / s;
        self.lp_a = 1.0; // `g/g`
        self.pgain = self.lp_k * self.hs2_k * self.ls_k * self.hs1_k;
        self.pack();
        self.set_parameter(0, self.p_predelay);
        self.set_parameter(1, self.p_decay);
        self.set_parameter(9, self.p_lowshelf);
        self.set_parameter(10, self.p_hidamp);
        self.set_early_delay();
        self.x1 = [0.0; LINES];
        self.x2 = [0.0; LINES];
        self.y1 = [0.0; LINES];
        self.y2 = [0.0; LINES];
        self.px = [0.0; 4];
        self.py = [0.0; 4];
        self.ap = [0.0; 20];
    }

    fn pack(&mut self) {
        self.pa = [self.hs2_a, self.lp_a, self.ls_a, self.hs1_a];
        self.pb = [self.hs2_b, self.lp_b, self.ls_b, self.hs1_b];
    }

    fn set_early_delay(&mut self) {
        let off = (self.sr * 0.005) as i32;
        for k in 0..32 {
            self.tap1[k] = 23999 - self.d_early[k];
            self.tap2[k] = 23999 - (self.d_early[k] + off);
        }
        self.ew = 0;
    }

    /// The output pre-delay's read index from the current pre-delay (ms).
    fn predelay_index(&mut self) {
        let s = (self.cur[0] / 1000.0) * self.sr;
        let s = if s < 1.0 { 1.0 } else { s };
        let d = self.pd_w - s as i32;
        self.pd_r = if d < 0 { d + PD_WRAP } else { d };
    }

    fn decay_coeffs(&mut self, d: f32) {
        let v = [d, d, d, 1.0 - d];
        let k: [f32; 4] = [6400.0, 1070.0, 2993.0, 757.0];
        let o = [556.0, 173.0, 1007.0, 968.0];
        for i in 0..4 {
            self.t60[i] = k[i].mul_add(v[i], o[i]);
        }
        self.hs1_g = exp10f((1.0 - d).mul_add(3.0, 0.0) / 20.0);
        self.high_shelf1();
    }

    fn low_shelf_coeffs(&mut self, v: f32) {
        self.ls_g = exp10f(v.mul_add(34.375, -28.75) / 20.0);
        self.low_shelf();
    }

    fn high_damp_coeffs(&mut self, v: f32) {
        self.f_hi = v.mul_add(4851.0, 6000.0);
        self.hs2_g = exp10f(v.mul_add(4.0, -4.0) / 20.0);
        self.high_shelf2();
    }

    fn high_shelf1(&mut self) {
        const K: f32 = c(0x3fd4_6d58);
        let (g, a, b) = shelf(
            self.hs1_g,
            K,
            c(0x4030_4548),
            1000.0,
            self.sr,
            |g, al| ((K - g).mul_add(al, g + K), (g + K).mul_add(al, K - g)),
            self.hs1_g * self.hs1_g + c(0xc030_4548),
        );
        self.hs1_k = g;
        self.hs1_a = a;
        self.hs1_b = b;
        self.pgain = g * ((self.hs2_k * self.lp_k) * self.ls_k);
        self.pack();
    }

    fn low_shelf(&mut self) {
        let gg = self.ls_g;
        // K = 1: sqrt(g·1) is sqrt(g).
        let (g, a, b) =
            shelf(gg, 1.0, 1.0, 160.0, self.sr, |g, al| ((g + -1.0).mul_add(al, g + 1.0), (g + 1.0).mul_add(al, g + -1.0)), 1.0 - gg * gg);
        self.ls_k = g;
        self.ls_a = a;
        self.ls_b = b;
        self.pgain = self.hs1_k * (g * (self.hs2_k * self.lp_k));
        self.pack();
    }

    fn high_shelf2(&mut self) {
        let (g, a, b) = shelf(
            self.hs2_g,
            1.0,
            1.0,
            5600.0,
            self.sr,
            |g, al| ((1.0 - g).mul_add(al, g + 1.0), (g + 1.0).mul_add(al, 1.0 - g)),
            self.hs2_g * self.hs2_g + -1.0,
        );
        self.hs2_k = g;
        self.hs2_a = a;
        self.hs2_b = b;
        self.pgain = ((g * self.lp_k) * self.ls_k) * self.hs1_k;
        self.pack();
    }

    /// `generateDampingFiltersForAllDelays()` (§4.6): per line, a 3-band T60 turned into
    /// two cascaded first-order shelves, multiplied out into one biquad.
    fn damping(&mut self) {
        let inv = 1.0 / self.sr;
        let k = inv * (NEG_LN_1000 * 1000.0);
        let e1 = k / self.t60[0];
        let e2 = k / self.t60[1];
        let e3 = k / self.t60[2];
        let r1 = inv * (2.0 * self.t60[3]);
        let r2 = inv * (2.0 * self.f_hi);
        let m1 = if 1.0 >= r1 { r1 } else { 1.0 };
        let m2 = if 1.0 >= r2 { r2 } else { 1.0 };
        let q4 = PI * 0.25;
        let h1 = (m1 * PI) * 0.5;
        let x1m = h1 - q4;
        let x1p = h1 + q4;
        let h2 = (m2 * PI) * 0.5;
        let x2m = h2 - q4;
        let x2p = h2 + q4;
        let sin1 = (x1m * sin_poly(x1m * x1m)) / (x1p * sin_poly(x1p * x1p));
        let t2 = (x2m * sin_poly(x2m * x2m)) / (x2p * sin_poly(x2p * x2p));
        for i in 0..LINES {
            let l = self.len[i];
            let g1 = pow2_approx((e1 * l) * LOG2_E);
            let g2 = pow2_approx((e2 * l) * LOG2_E);
            let g3 = pow2_approx((e3 * l) * LOG2_E);
            // Low shelf from g1 (mid) to g3 (high) … (lane order as in the binary)
            let sq = (g1 * g3).sqrt();
            let den = (g1 * g1 + g3 * g3) - 2.0 * (sq * sq);
            let z1 = den == 0.0;
            let q1 = (g3 * g3 - g1 * g1) / if z1 { 1.0 } else { den };
            let v = q1 * q1 - 1.0;
            let s = (if v > 0.0 - v { v } else { 0.0 - v }).sqrt();
            let s = if z1 { 0.0 } else { s };
            let alpha1 = if z1 { 0.0 } else { q1 + if q1 >= 0.0 { -s } else { s } };
            let r = g2 / g3;
            let sq2 = (1.0 * r).sqrt();
            let den2 = (1.0 + r * r) - 2.0 * (sq2 * sq2);
            let z2 = den2 == 0.0;
            let q2 = (r * r - 1.0) / if z2 { 1.0 } else { den2 };
            let v2 = q2 * q2 - 1.0;
            let s2 = (if v2 > 0.0 - v2 { v2 } else { 0.0 - v2 }).sqrt();
            let s2 = if z2 { 0.0 } else { s2 };
            let alpha2 = if z2 { 0.0 } else { q2 + if q2 >= 0.0 { -s2 } else { s2 } };
            let a1 = ((g1 + g3) + (g1 - g3) * alpha1) * 0.5;
            let bb1 = ((g1 - g3) + (g1 + g3) * alpha1) * 0.5;
            let t1 = sin1;
            let n1 = 1.0 / (1.0 + alpha1 * t1);
            let u0 = (a1 + bb1 * t1) * n1;
            let u1 = (bb1 + a1 * t1) * n1;
            let d1 = (alpha1 + t1) * n1;
            let a2 = ((1.0 + r) + (1.0 - r) * alpha2) * 0.5;
            let bb2 = ((1.0 - r) + (1.0 + r) * alpha2) * 0.5;
            let n2 = 1.0 / (1.0 + alpha2 * t2);
            let w0 = (a2 + bb2 * t2) * n2;
            let w1 = (bb2 + a2 * t2) * n2;
            let d2 = (alpha2 + t2) * n2;
            self.b0[i] = u0 * w0;
            self.b1[i] = u0 * w1 + u1 * w0;
            self.b2[i] = u1 * w1;
            self.a1[i] = 1.0 * d2 + 1.0 * d1;
            self.a2[i] = d1 * d2;
        }
    }

    /// `setParameter(idx, v)`.
    pub(crate) fn set_parameter(&mut self, idx: u32, v: f32) {
        match idx {
            0 => {
                self.p_predelay = v;
                self.tgt[0] = v.mul_add(250.0, 0.0);
                self.dirty = true;
                self.predelay_index();
            }
            1 => {
                self.p_decay = v;
                self.tgt[1] = v;
                self.dirty = true;
                self.decay_coeffs(self.cur[1]);
                self.damping();
            }
            7 => {
                self.p_mix = v;
                self.tgt[2] = v;
                self.dirty = true;
            }
            8 => self.wet_gain = v,
            9 => {
                self.p_lowshelf = v;
                self.tgt[3] = 1.0 - v;
                self.dirty = true;
                self.low_shelf_coeffs(self.cur[3]);
            }
            10 => {
                self.p_hidamp = v;
                self.tgt[4] = 1.0 - v;
                self.dirty = true;
                self.high_damp_coeffs(self.cur[4]);
                self.damping();
            }
            13 => {
                self.p_width = v;
                self.tgt[5] = v;
                self.dirty = true;
            }
            _ => {}
        }
        if self.immediate {
            self.cur = self.tgt;
            self.dirty = true;
        }
    }

    /// `onReset()` = `initBuffersAndPointers()` and clearing the filter states.
    pub(crate) fn reset(&mut self) {
        self.early.fill(0.0);
        self.out = [0.0; LINES];
        self.lines.fill(0.0);
        self.r = [0; LINES];
        self.ew = 0;
        self.tap1 = [0; 32];
        self.tap2 = [0; 32];
        for i in 0..LINES {
            self.w[i] = (self.len[i] as i32).wrapping_add(-1);
        }
        for b in &mut self.pd {
            b.fill(0.0);
        }
        // The filters' own coefficients go back to unity; the packed ones stay.
        self.lp_k = 1.0;
        self.lp_a = 0.0;
        self.lp_b = 0.0;
        self.hs1_k = 1.0;
        self.hs1_a = 0.0;
        self.hs1_b = 0.0;
        self.ls_k = 1.0;
        self.ls_a = 0.0;
        self.ls_b = 0.0;
        self.hs2_k = 1.0;
        self.hs2_a = 0.0;
        self.hs2_b = 0.0;
        self.b0 = [1.0; LINES];
        self.b1 = [0.0; LINES];
        self.b2 = [0.0; LINES];
        self.a1 = [0.0; LINES];
        self.a2 = [0.0; LINES];
        self.set_early_delay();
        self.predelay_index();
        self.damping();
        self.x1 = [0.0; LINES];
        self.x2 = [0.0; LINES];
        self.y1 = [0.0; LINES];
        self.y2 = [0.0; LINES];
        self.px = [0.0; 4];
        self.py = [0.0; 4];
        self.ap = [0.0; 20];
    }

    /// `onResume()`: from now on parameters ramp.
    pub(crate) fn resume(&mut self) {
        self.immediate = false;
    }

    /// `processControlRate()`, every 128 samples: start or finish the parameter ramps.
    pub(crate) fn control(&mut self) {
        if self.dirty {
            self.dirty = false;
            let mut sum = 0.0;
            for i in 0..8 {
                let d = (self.tgt[i] - self.cur[i]) * self.kinc;
                self.inc[i] = d;
                if d * d < 1e-15 {
                    self.inc[i] = 0.0;
                    self.cur[i] = self.tgt[i];
                } else {
                    sum += d * d;
                }
            }
            if sum > 0.0 {
                self.countdown = self.ticks;
                self.ramping = true;
            }
        } else {
            self.countdown -= 1;
            if self.countdown == 0 {
                self.cur = self.tgt;
                self.inc = [0.0; 8];
                self.ramping = false;
            }
        }
    }

    /// One block of `processAudioImpl<false>` (stereo in and out), in place.
    pub(crate) fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            (*xl, *xr) = self.tick(*xl, *xr);
        }
    }

    fn ramp(&mut self) {
        for i in 0..8 {
            self.cur[i] += self.inc[i];
        }
        if !self.ramping {
            return;
        }
        if self.inc[1] != 0.0 {
            let d = self.cur[1];
            self.decay_coeffs(d);
            self.damping();
        }
        if self.inc[3] != 0.0 {
            self.low_shelf_coeffs(self.cur[3]);
        }
        if self.inc[4] != 0.0 {
            self.high_damp_coeffs(self.cur[4]);
            self.damping();
        }
        if self.inc[0] != 0.0 {
            self.predelay_index();
        }
        if self.inc[5] != 0.0 {
            let w = self.cur[5];
            let s1 = w.mul_add(-2.0, 1.0) * QUARTER_PI;
            let x = [s1.max(0.0), (1.0 - w) * QUARTER_PI, (w + 1.0) * QUARTER_PI, (w.mul_add(2.0, 1.0) * QUARTER_PI).min(HALF_PI)];
            for i in 0..4 {
                let x2 = x[i] * x[i];
                self.sinv[i] = x[i] * sin_poly(x2);
                self.cosv[i] = cos_poly(x2);
            }
        }
    }

    fn tick(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        self.ramp();
        // Mono sum, then 20 allpasses.
        let mut x = 0.5 * ((0.0 + in_l) + in_r);
        for s in &mut self.ap {
            let v = s.mul_add(0.9, x);
            x = v.mul_add(-0.9, *s);
            *s = v;
        }
        // The four first-order filters as a pipeline: lane 3 takes the input.
        let nx = [self.py[1], self.py[2], self.py[3], x * self.pgain];
        let mut ny = [0.0; 4];
        for i in 0..4 {
            ny[i] = (nx[i] + self.pa[i] * self.px[i]) - self.py[i] * self.pb[i];
        }
        self.px = nx;
        self.py = ny;
        let ew = self.ew;
        self.early[ew as usize] = ny[0];
        self.ew = if ew > 23998 { 0 } else { ew + 1 };
        // Early taps: two sets of 32, the first feeds the lines, the second the output.
        let mut acc = [0.0f32; 16];
        let mut direct = [0.0f32; 4];
        for g in 0..8 {
            for lane in 0..4 {
                let k = g * 4 + lane;
                let j = (g & 3) * 4 + lane;
                let i1 = self.tap1[k];
                acc[j] += self.early[i1 as usize];
                self.tap1[k] = if i1 > 23998 { 0 } else { i1 + 1 };
                let coef = EARLY_GAIN * c(TAP_W[k]);
                let i2 = self.tap2[k];
                direct[lane] += self.early[i2 as usize] * coef;
                self.tap2[k] = if i2 > 23998 { 0 } else { i2 + 1 };
            }
        }
        // Damping biquads on the lines' outputs, then the early taps join.
        let mut u = [0.0f32; LINES];
        for i in 0..LINES {
            let xi = self.out[i];
            let y = (((self.b0[i] * xi + self.b1[i] * self.x1[i]) + self.b2[i] * self.x2[i]) - self.a1[i] * self.y1[i])
                - self.a2[i] * self.y2[i];
            self.x2[i] = self.x1[i];
            self.x1[i] = xi;
            self.y2[i] = self.y1[i];
            self.y1[i] = y;
            u[i] = 0.25 * (y + INJECT[i] * acc[i]);
        }
        // A 16-point Hadamard transform, as the binary orders it.
        let h = hadamard16(&u);
        let mut sum = [0.0f32; 4];
        for (set, outs) in [[0, 1, 2, 3], [4, 5, 6, 7], [8, 9, 10, 11], [12, 13, 14, 15]].iter().enumerate() {
            for &i in outs {
                let w = self.w[i];
                self.lines[i * LINE_LEN + w as usize] = h[i];
                self.w[i] = if w > 21198 { 0 } else { w + 1 };
            }
            for &i in outs {
                let rr = self.r[i];
                self.out[i] = self.lines[i * LINE_LEN + rr as usize];
                self.r[i] = if rr > 21198 { 0 } else { rr + 1 };
            }
            for lane in 0..4 {
                sum[lane] = if set == 0 { self.out[outs[lane]] + 0.0 } else { sum[lane] + self.out[outs[lane]] };
            }
        }
        let mix = self.cur[2];
        let mut v3 = [0.0f32; 4];
        for i in 0..4 {
            v3[i] = direct[i] + sum[i];
        }
        let hs = |a: [f32; 4], b: [f32; 4]| (a[0] * b[0] + a[1] * b[1]) + (a[2] * b[2] + a[3] * b[3]);
        let wet = [hs(v3, self.cosv), hs(v3, self.sinv)];
        let dry_in = [in_l, in_l, in_r, in_r];
        let dry = [hs(self.cosv, dry_in), hs(self.sinv, dry_in)];
        let kd = (1.0 - mix) * c(0x3f00_4dce);
        let kw = (mix * self.wet_gain) * c(0x3d2c_ba15);
        let mut o = [0.0f32; 2];
        for ch in 0..2 {
            self.pd[ch][self.pd_w as usize] = wet[ch];
            let s = self.pd[ch][self.pd_r as usize];
            o[ch] = kw.mul_add(s, kd * dry[ch]);
        }
        self.pd_w = if self.pd_w > PD_WRAP - 1 { 0 } else { self.pd_w + 1 };
        self.pd_r = if self.pd_r > PD_WRAP - 1 { 0 } else { self.pd_r + 1 };
        (o[0], o[1])
    }
}

/// The network's 16-point Hadamard transform (unnormalised), in the binary's order of
/// sums; output `i` goes to line `i`.
fn hadamard16(u: &[f32; 16]) -> [f32; 16] {
    // Stage 1 and 2 within each group of four.
    let quad = |a: &[f32]| {
        let (p, m, p2, m2) = (a[0] + a[1], a[0] - a[1], a[2] + a[3], a[2] - a[3]);
        [p + p2, m + m2, p - p2, m - m2]
    };
    let a = quad(&u[0..4]);
    let b = quad(&u[4..8]);
    let cc = quad(&u[8..12]);
    let d = quad(&u[12..16]);
    let mut ab_p = [0.0; 4];
    let mut ab_m = [0.0; 4];
    let mut cd_p = [0.0; 4];
    let mut cd_m = [0.0; 4];
    for k in 0..4 {
        ab_p[k] = a[k] + b[k];
        ab_m[k] = a[k] - b[k];
        cd_p[k] = cc[k] + d[k];
        cd_m[k] = cc[k] - d[k];
    }
    let mut h = [0.0; 16];
    for k in 0..4 {
        h[k] = ab_p[k] + cd_p[k];
        h[4 + k] = ab_m[k] + cd_m[k];
        h[8 + k] = ab_p[k] - cd_p[k];
        h[12 + k] = ab_m[k] - cd_m[k];
    }
    h
}
