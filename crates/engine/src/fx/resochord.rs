//! Maschine's Resochord (docs/fx-resochord.md): `RT::ResochordProcessor` →
//! `FXCORE::ResochordCore`, a compiled Reaktor-Core kernel. Six tuned comb resonators
//! (three per channel), each a delay line one pitch period long with a fixed 182 Hz
//! shelf and a Brightness low-pass in its loop and a soft clipper on its output. Their
//! pitches come from Tune plus a Spread detune (String mode) or a chord table (Chord
//! mode, with a slow triangle vibrato). The resonator inputs are summed, DC-blocked,
//! shaped by two fixed filters and levelled by an automatic gain, then mixed with the
//! dry input. Control values update every `sr/8820` samples and glide over 20 ms.
//!
//! The kernel is ported as a flat array of its cells (`f32`, or `i32` bit patterns for
//! counters, flags and tap indices), indexed as in the kernel (byte offset / 4), so the
//! code can follow the assembly operation by operation. The kernel has no fused
//! multiply-adds, and the port keeps its operation order (which Ghidra's output does not
//! always show). The output can match Maschine bit for bit.

// `!(x < 1.0)` mirrors the kernel's `b.lt` tests, where NaN falls through.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::{Effect, param};

/// Kernel cells (the kernel is 0x306c8 bytes; the event-list area after the lines is
/// replaced by local flags).
const K_LEN: usize = 0xc181;
/// The kernel's denormal guard (`0x219392ef`).
const DC: f32 = f32::from_bits(0x2193_92ef); // 1e-18
/// 1/12.0006: semitones to octaves.
const OCT: f32 = 0.083_329_2;
/// MIDI note 0 in Hz.
const NOTE0: f32 = 8.177_42;
const PI: f32 = f32::from_bits(0x4049_0fd0); // 3.14159
const HALF_PI: f32 = f32::from_bits(0x3fc9_0ff9); // 1.5708
/// −1/π: atan to cycles.
const NEG_INV_PI: f32 = f32::from_bits(0xbea2_f987); // −0.31831
/// 2π and ln 2 as the kernel writes them.
const TWO_PI: f32 = f32::from_bits(0x40c9_0fe4); // 6.28319
const LN2: f32 = f32::from_bits(0x3f31_7215); // 0.693147
/// tan Taylor coefficients.
const T3: f32 = 0.333_333;
const T5: f32 = 0.133_333;
/// atan polynomial (in x², highest power first).
const ATAN: [f32; 8] = [0.002_866_23, -0.016_165_7, 0.042_909_6, -0.075_289_6, 0.106_563, -0.142_089, 0.199_936, -0.333_331];

// Input cells (written by the processor's parameter callbacks).
const IN_L: usize = 0x3;
const IN_R: usize = 0x8;
const OUT_L: usize = 0x11;
const OUT_R: usize = 0x12;
const MIX: usize = 0x4;
const FEEDBACK: usize = 0x6;
const DECAY: usize = 0x9;
const BRIGHTNESS: usize = 0xb;
const SPREAD: usize = 0xd;
const MODE: usize = 0xf;
const STYLE: usize = 0x13;
const CHORD: usize = 0x15;
const TUNE: usize = 0x17;

/// Offsets in a voice's pitch block (`Voice::p`).
const PER: usize = 1; // period, samples
const PER1: usize = 2; // period − 1
const B136: usize = 3; // 136 − pitch (Brightness key tracking)
const MAXF: usize = 4; // 0.9·sr
const PISR: usize = 5; // π/sr
const BRT: usize = 6; // π / tan(π·f_b/sr)
const PTAN: usize = 7; // π / tan(π/period)
const ATN: usize = 8; // phase-delay correction, cycles
const COMP: usize = 9; // … in samples
const G: usize = 10; // 182 Hz shelf: g, g − 1, state, 1/(g + 1), mix
const GM1: usize = 11;
const S1: usize = 12;
const INV: usize = 13;
const SHELF: usize = 14;
const S2: usize = 15; // Brightness low-pass: state, max f, π/sr, g, g − 1, 1/(g + 1)
const MAXF2: usize = 16;
const PISR2: usize = 17;
const TB: usize = 18;
const TBM1: usize = 19;
const TBINV: usize = 20;
const TAP: usize = 21; // n, n + 2, n − 1, n + 1 (i32)
const FR: usize = 25; // f, f + 1, f − 1
const LO: usize = 28; // clip floor

/// One resonator: the kernel cells it owns (§3.3).
#[derive(Clone, Copy)]
struct Voice {
    /// Pitch ramp: current, start, step, position.
    r: usize,
    /// Pitch block (offsets above); `p` itself is the ramped pitch.
    p: usize,
    /// Delay line (8192 cells).
    line: usize,
    /// Soft-clipped output, fed back into both resonators' inputs of its channel.
    out: usize,
    /// Its bit in the pitch-event mask (+0x22c).
    bit: i32,
    /// Chord table value, × chord gain, + vibrato.
    tab: usize,
    cv: usize,
    lfo: usize,
    cl: usize,
    /// Spread offset, and tune + spread.
    sp: usize,
    ns: usize,
    /// Spread multiplier (semitones per Spread²).
    spread: f32,
}

const fn voice(r: usize, p: usize, line: usize, out: usize, bit: i32, cells: [usize; 6], spread: f32) -> Voice {
    let [tab, cv, lfo, cl, sp, ns] = cells;
    Voice { r, p, line, out, bit, tab, cv, lfo, cl, sp, ns, spread }
}

/// The six voices in chord-table order. A, B, C feed the left output and D, E, F the
/// right. Spread detunes the left ones down and the right ones up.
const A: Voice = voice(0x9d, 0xa1, 0x181, 0x42, 1, [0x8f, 0x91, 0x96, 0x97, 0x98, 0x9c], -0.222);
const D: Voice = voice(0xcb, 0x120, 0x6181, 0x53, 2, [0xbf, 0xc6, 0xc7, 0xc8, 0xc9, 0xca], 0.222);
const B: Voice = voice(0xcf, 0xd3, 0x2181, 0x44, 4, [0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5], -0.444);
const E: Voice = voice(0xfb, 0x13d, 0x8181, 0x54, 8, [0xf0, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa], 0.444);
const C: Voice = voice(0xff, 0x103, 0x4181, 0x45, 16, [0xf1, 0xf2, 0x96, 0xf3, 0xf4, 0xf5], -0.666);
const F: Voice = voice(0x15f, 0x163, 0xa181, 0x55, 32, [0x15a, 0x15b, 0xc7, 0x15c, 0x15d, 0x15e], 0.666);
const VOICES: [Voice; 6] = [A, D, B, E, C, F];

/// `s_06C064A8` (0x10dc36270): chord voicings in semitones, `[style][chord][voice]`
/// (voices in `VOICES` order), copied as stored.
const CHORDS: [f32; 150] = [
    -12.0,
    0.0,
    0.0,
    3.928,
    3.9064,
    3.928,
    -12.0,
    0.0,
    3.9352,
    3.9064,
    7.0,
    7.0,
    -12.0,
    0.0,
    3.9352,
    7.0216,
    11.0,
    11.0,
    -12.0,
    0.0,
    3.9352,
    7.0,
    11.0,
    14.0,
    -12.0,
    0.0,
    3.9352,
    7.0,
    14.0,
    18.0,
    -12.0,
    0.0,
    0.0,
    3.0,
    3.0,
    3.0,
    -12.0,
    0.0,
    3.0,
    3.0,
    7.0,
    7.0,
    -12.0,
    0.0,
    3.0,
    7.0216,
    10.0,
    10.0,
    -12.0,
    0.0,
    3.0,
    7.0,
    10.0,
    14.0,
    -12.0,
    0.0,
    3.0,
    7.0,
    10.0,
    17.0,
    -12.0,
    0.0,
    4.0,
    7.0,
    10.0,
    10.0,
    -12.0,
    0.0,
    4.0,
    8.0,
    8.0,
    8.0,
    -12.0,
    0.0,
    3.0,
    6.0,
    6.0,
    6.0,
    -12.0,
    0.0,
    3.0,
    6.0,
    9.0,
    9.0,
    -12.0,
    0.0,
    3.0,
    6.0,
    11.0,
    11.0,
    0.0,
    7.0,
    14.0,
    4.0,
    11.0,
    12.0648,
    0.0,
    5.0288,
    14.0504,
    2.0,
    7.0,
    21.0,
    -0.00719833,
    8.9928,
    23.0,
    0.0143967,
    14.1152,
    19.0792,
    -5.0,
    7.0,
    2.2088,
    2.0144,
    25.0,
    14.0,
    0.0720024,
    -0.0503998,
    -0.115204,
    2.9856,
    4.99281,
    12.0216,
    -14.0,
    -7.0,
    0.0,
    7.0,
    14.0,
    14.0,
    -12.0,
    0.0,
    15.0,
    9.9936,
    7.0,
    15.0,
    -12.0,
    3.0,
    7.0216,
    9.0,
    12.072,
    12.0,
    -12.0,
    3.0,
    7.0216,
    10.0,
    14.0,
    17.0,
    -12.0,
    2.0,
    7.0,
    9.0,
    12.0,
    14.0,
];
/// `s_0711FA78` (0x10dc36084): input gain by Feedback (41 steps; × 1/3 in use).
const IN_GAIN: [f32; 41] = [
    0.85, 0.868007, 0.855867, 0.843726, 0.831585, 0.788573, 0.74556, 0.702548, 0.659535, 0.616523, 0.577118, 0.530498, 0.48582, 0.450077,
    0.420843, 0.394523, 0.364462, 0.337801, 0.298239, 0.27, 0.26, 0.33, 0.4, 0.46, 0.52, 0.600427, 0.638014, 0.658831, 0.676637, 0.684732,
    0.687775, 0.688131, 0.688131, 0.688131, 0.688131, 0.688131, 0.688131, 0.683759, 0.683759, 0.683759, 0.683759,
];
/// `s_0711F568` (0x10dc36128): loop gain by Feedback.
const FB_GAIN: [f32; 41] = [
    0.62, 0.71, 0.8, 0.868946, 0.896457, 0.9167, 0.932256, 0.945, 0.955, 0.96, 0.965, 0.975, 0.987361, 0.989995, 0.992253, 0.994658,
    0.996415, 0.99768, 0.999028, 1.0002, 1.0003, 1.0015, 1.0036, 1.00663, 1.00923, 1.01278, 1.01657, 1.02088, 1.02622, 1.04, 1.05, 1.06,
    1.07, 1.079, 1.08, 1.10101, 1.11353, 1.127, 1.13928, 1.15293, 1.2,
];
/// `s_0711EDD0` (0x10dc361cc): clip level by Feedback (× ½ in use).
const CLIP: [f32; 41] = [
    50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 25.0762,
    12.4007, 3.2857, 1.92178, 1.29463, 0.977215, 0.719886, 0.569777, 0.414609, 0.315882, 0.270566, 0.238204, 0.215546, 0.196124, 0.178321,
    0.163755, 0.148373, 0.133808, 0.118438, 0.103871, 0.09,
];

/// `round()` as the kernel does it: add ±0.5 (by the sign of the `f32`) in `f64`, truncate.
fn rnd(x: f32) -> i32 {
    let h = if x < 0.0 { -0.5 } else { 0.5 };
    (h + x as f64) as i32
}

/// `2^n` built in the exponent bits.
fn pow2(n: i32) -> f32 {
    f32::from_bits(0x3f80_0000u32.wrapping_add((n as u32) << 23))
}

/// Entry `i` of the kernel's octave table `s_04D103F0` (shared with Chorus):
/// `2^(i/1021 − 0.5)` to 6 significant digits, with the last entry stored as √2 and seven
/// near-ties rounded the other way.
fn exp2_entry(i: usize) -> f32 {
    match i {
        106 => return 0.759867,
        213 => return 0.817119,
        225 => return 0.823803,
        287 => return 0.859218,
        324 => return 0.881074,
        382 => return 0.91646,
        673 => return 1.11663,
        1022.. => return f32::from_bits(0x3fb5_04d5), // 1.41421
        _ => {}
    }
    let v = (i as f64 / 1021.0 - 0.5).exp2();
    let p = 10f64.powi(5 - v.log10().floor() as i32);
    ((v * p).round() / p) as f32
}

/// `2^x` by the octave table (coefficients only).
fn exp2_tab(x: f32) -> f32 {
    let n = rnd(x);
    let y = ((x - n as f32) + 0.5) * 1021.0;
    let k = rnd(y + -0.5);
    let fr = y - k as f32;
    let i = (k as u32 & 0x3ff) as usize;
    ((1.0 - fr) * exp2_entry(i) + fr * exp2_entry(i + 1)) * pow2(n)
}

/// Pitch (semitones) to Hz: a cubic `2^x` for the fraction of the octave.
fn note_hz(x: f32) -> f32 {
    let p = x * OCT;
    let n = rnd(p);
    let f = p - n as f32;
    (f * ((f * (f * 0.056 + 0.242)) + 0.693) + 1.0) * pow2(n) * NOTE0
}

/// The Brightness cutoff's `2^x`: a quadratic for the fraction.
fn bright_hz(x: f32) -> f32 {
    let p = x * OCT;
    let n = rnd(p);
    let f = p - n as f32;
    (f * (f * 0.248 + 0.708) + 1.0) * pow2(n) * NOTE0
}

/// `tan x ≈ x + x³/3 + 2x⁵/15`.
fn tan_approx(x: f32) -> f32 {
    let x2 = x * x;
    let x3 = x * x2;
    let x5 = x2 * x3;
    x + (x3 * T3 + x5 * T5)
}

/// `atan x` on |x| ≤ 1: `x·(1 + x²·P(x²))`, Horner as in the kernel.
fn atan_series(x: f32) -> f32 {
    let x2 = x * x;
    let mut p = x2 * ATAN[0];
    for c in &ATAN[1..] {
        p = x2 * (p + c);
    }
    x * (p + 1.0)
}

/// `atan r`: the series on |r| ≤ 1, `±π/2 − atan(1/r)` outside.
fn atan_approx(r: f32) -> f32 {
    if r > 1.0 {
        HALF_PI - atan_series(1.0 / r)
    } else if r > -1.0 {
        atan_series(r)
    } else {
        -HALF_PI - atan_series(1.0 / r)
    }
}

/// `ResochordCore`: the kernel's cells.
struct Core {
    k: Vec<f32>,
}

impl Core {
    fn new() -> Self {
        Core { k: vec![0.0; K_LEN] }
    }

    fn gi(&self, i: usize) -> i32 {
        self.k[i].to_bits() as i32
    }

    fn si(&mut self, i: usize, v: i32) {
        self.k[i] = f32::from_bits(v as u32);
    }

    fn or(&mut self, i: usize, m: i32) {
        let v = self.gi(i) | m;
        self.si(i, v);
    }

    fn clear(&mut self, i: usize, m: i32) {
        let v = self.gi(i) & !m;
        self.si(i, v);
    }

    /// Restart a voice's pitch ramp towards `t` (it glides there over 20 ms, §3.2).
    fn glide(&mut self, v: Voice, t: f32) {
        let k = &mut self.k;
        k[v.r + 1] = k[v.r];
        k[v.r + 2] = t - k[v.r];
        k[v.r + 3] = 0.0;
    }

    /// Step a voice's pitch ramp and derive its period (control rate, §3.2).
    fn pitch(&mut self, v: Voice, inc: f32) {
        let mut pos = self.k[v.r + 3] + inc;
        if !(pos < 1.0) {
            self.clear(0x8b, v.bit);
            pos = 1.0;
        }
        let k = &mut self.k;
        let p = v.p;
        k[v.r + 3] = pos;
        let x = pos * k[v.r + 2] + k[v.r + 1];
        k[p] = x;
        k[v.r] = x;
        let per = k[0] / note_hz(x);
        k[p + PER] = per;
        k[p + PER1] = per + -1.0;
        k[p + B136] = 136.0 - x;
        let t = if per < 0.0 { f32::INFINITY } else { PI / per };
        k[p + PTAN] = PI / tan_approx(t);
    }

    /// A voice's Brightness low-pass and loop-delay correction; returns the delay.
    fn color(&mut self, v: Voice) -> f32 {
        let k = &mut self.k;
        let p = v.p;
        let fb = bright_hz(k[p] + k[p + B136] * k[0x87]);
        let a = if fb < 1.0 { 1.0 } else { fb };
        let a = if fb > k[p + MAXF] { k[p + MAXF] } else { a };
        let brt = PI / tan_approx(k[p + PISR] * a);
        k[p + BRT] = brt;
        let b = if fb < 0.0 { 0.0 } else { fb };
        let b = if b > k[p + MAXF2] { k[p + MAXF2] } else { b };
        let tb = tan_approx(k[p + PISR2] * b);
        k[p + TB] = tb;
        k[p + TBINV] = 1.0 / (tb + 1.0);
        k[p + TBM1] = tb + -1.0;
        let at = atan_approx(brt / k[p + PTAN]) * NEG_INV_PI;
        k[p + ATN] = at;
        let comp = at * k[p + PER] * 0.5;
        k[p + COMP] = comp;
        k[p + PER1] + comp
    }

    /// Tap positions for a delay of `d` samples (cubic read around it, §3.4).
    fn taps(&mut self, v: Voice, d: f32) {
        let d = d.clamp(0.0, 8190.0);
        let n = rnd(d + -0.5);
        let f = d - n as f32;
        let p = v.p;
        self.si(p + TAP, n);
        self.si(p + TAP + 1, n + 2);
        self.si(p + TAP + 2, n - 1);
        self.si(p + TAP + 3, n + 1);
        self.k[p + FR] = f;
        self.k[p + FR + 1] = f + 1.0;
        self.k[p + FR + 2] = f + -1.0;
    }

    /// The 182 Hz shelf of a voice's loop on input `x`; returns its correction.
    fn shelf(&mut self, v: Voice, x: f32) -> f32 {
        let k = &mut self.k;
        let p = v.p;
        let s = k[p + S1];
        let u = (x - s * k[p + GM1]) * k[p + INV];
        k[p + S1] = u + DC;
        (s + u) * k[p + G] * k[p + SHELF]
    }

    /// The Brightness low-pass on `x + w`, written into the line at `wi`.
    fn write(&mut self, v: Voice, x: f32, w: f32, wi: usize) {
        let k = &mut self.k;
        let p = v.p;
        let s = k[p + S2];
        let y = k[p + TBINV] * ((x + w) - s * k[p + TBM1]);
        k[p + S2] = y + DC;
        k[v.line + wi] = k[p + TB] * (s + y);
    }

    /// Read a voice's line at its taps (`w`: write position) and soft-clip it into its
    /// output cell.
    fn read(&mut self, v: Voice, w: i32) {
        let p = v.p;
        let at = |c: &Core, t: usize| c.k[v.line + (w.wrapping_sub(c.gi(p + TAP + t)) & 0x1fff) as usize];
        let d0 = at(self, 0);
        let a2 = at(self, 1);
        let bm = at(self, 2);
        let cp = at(self, 3);
        let k = &mut self.k;
        let h = (cp - bm) * 0.5;
        let e = cp - d0;
        let g = e - h;
        let q = ((a2 - bm) * T3 - h) - g;
        let y = d0 + k[p + FR] * (e + k[p + FR + 2] * (g + k[p + FR + 1] * (q * 0.5)));
        let hi = k[0x88];
        let c = if y <= hi { if y < k[0x89] { k[p + LO] } else { y } } else { hi };
        k[v.out] = c * (1.0 - c.abs() * k[0xbe]) + DC;
    }

    /// A ramp of the `[0x22]` group (bits 1–3): position at `pos`, step, start before it.
    fn ramp22(&mut self, pos: usize, bit: i32, inc: f32) -> f32 {
        let mut x = self.k[pos] + inc;
        if !(x < 1.0) {
            self.clear(0x22, bit);
            x = 1.0;
        }
        self.k[pos] = x;
        x * self.k[pos - 1] + self.k[pos - 2]
    }

    /// One sample (`processAudio`). `init` completes every glide at once, as `endInit`
    /// does.
    fn tick(&mut self, init: bool) {
        let in_l = self.k[IN_L];
        let in_r = self.k[IN_R];
        let w = self.gi(0x8a);
        let wi = (w as u32).min(0x1fff) as usize;
        self.si(0x8a, (w + 1) & 0x1fff);
        let ctl = self.gi(0x19) >= self.gi(0x1a);
        let (inc_v, inc_m, inc_c) = if init { (1.0, 1.0, 1.0) } else { (self.k[0x8c], self.k[0x1c], self.k[0x23]) };
        let mut lfo = false;
        let mut ev = [false; 6];
        let mut ev_b = false;
        let mut delay = [0.0f32; 6];
        let (mut old8b, mut old1b, mut old22) = (0, 0, 0);

        // Control block, first part (§3.2).
        if ctl {
            old8b = self.gi(0x8b);
            let cnt = self.gi(0x92);
            lfo = cnt > 0x30;
            let new_cnt = if lfo { 0 } else { cnt + 1 };
            if lfo {
                // Vibrato: a triangle every 50 control periods, ±depth semitones; the
                // second voice of each channel gets it a quarter period later.
                let k = &mut self.k;
                let p = k[0x94] + k[0x93];
                let n = rnd(p) as f32;
                let ph = p - n;
                k[0x94] = ph;
                let tri0 = 1.0 - (p - n).abs() * 4.0;
                let depth = k[0x95];
                let neg0 = -(tri0 * depth);
                k[0xc7] = neg0;
                k[0x15c] = k[0x15b] + neg0;
                let t = k[0x15e] + k[0x15c];
                self.glide(F, t);
                let k = &mut self.k;
                let pos0 = depth * tri0;
                k[0x96] = pos0;
                k[0xf3] = pos0 + k[0xf2];
                let t = k[0xf3] + k[0xf5];
                self.glide(C, t);
                let k = &mut self.k;
                let ph2 = ph + 0.25;
                let n2 = rnd(ph2) as f32;
                let tri1 = 1.0 - (ph2 - n2).abs() * 4.0;
                let neg1 = -(tri1 * depth);
                k[0xf7] = neg1;
                k[0xf8] = neg1 + k[0xf6];
                let t = k[0xf8] + k[0xfa];
                self.glide(E, t);
                let k = &mut self.k;
                let pos1 = depth * tri1;
                k[0xc2] = pos1;
                k[0xc3] = pos1 + k[0xc1];
                let t = k[0xc3] + k[0xc5];
                self.glide(B, t);
                let k = &mut self.k;
                k[0xc8] = neg0 + k[0xc6];
                let t = k[0xc8] + k[0xca];
                self.glide(D, t);
                let k = &mut self.k;
                k[0x97] = pos0 + k[0x91];
                let t = k[0x97] + k[0x9c];
                self.glide(A, t);
                self.si(0x8b, old8b | 1);
            }

            // Input envelope (Decay): a hold counter and an asymmetric smoother.
            let peak = if in_l.abs() > in_r.abs() { in_l.abs() } else { in_r.abs() };
            let (lim, cnt2) = if peak > self.k[0x24] {
                let lim = self.gi(0x26);
                self.si(0x27, lim);
                (lim, 0)
            } else {
                (self.gi(0x27), self.gi(0x28))
            };
            self.si(0x28, cnt2 + 1);
            let gate = if lim > cnt2 { 1.0 } else { 0.0 };
            self.si(0x92, new_cnt);
            let k = &mut self.k;
            let d = gate - k[0x29];
            let c = if d > 0.0 { k[0x2a] } else { k[0x2d] };
            let e = k[0x29] + (d + DC) * c;
            k[0x29] = e;
            k[0x2e] = e * (2.0 - e);
            old22 = self.gi(0x22);
            self.si(0x22, old22 | 1);
            let (k2f, k30) = (self.k[0x2f], self.k[0x30]);
            self.k[0x31] = 0.0;
            if peak > 0.000_1 {
                self.si(0x66, 0);
                let h = self.gi(0x64);
                self.si(0x65, h);
            }
            old1b = self.gi(0x1b);

            if old8b != 0 && self.gi(0x8b) & A.bit != 0 {
                ev[0] = true;
                self.pitch(A, inc_v);
            }
            if old22 != 0 {
                // The envelope-scaled Brightness, smoothed.
                ev_b = true;
                let mut c = if init { 1.0 } else { inc_c + 0.0 };
                if !(c < 1.0) {
                    self.si(0x22, old22 & !1);
                    c = 1.0;
                }
                let k = &mut self.k;
                let v = k30 + (k[0x2e] * k2f - k30) * c;
                k[0x87] = v;
                k[0x30] = v;
                k[0x31] = c;
            }
            if ev_b || ev[0] {
                delay[0] = self.color(A);
            }
            if old1b != 0 && self.gi(0x1b) & 1 != 0 {
                // Dry gain glide.
                let mut pos = self.k[0x20] + inc_m;
                if !(pos < 1.0) {
                    self.clear(0x1b, 1);
                    pos = 1.0;
                }
                let k = &mut self.k;
                let v = pos * k[0x1f] + k[0x1e];
                k[0x20] = pos;
                k[0x21] = v;
                k[0x1d] = v;
            }
            if lfo {
                self.or(0x8b, 2);
            }
            if old8b != 0 && self.gi(0x8b) & D.bit != 0 {
                ev[1] = true;
                self.pitch(D, inc_v);
            }
            if ev_b || ev[1] {
                delay[1] = self.color(D);
            }
            if old22 != 0 && self.gi(0x22) & 2 != 0 {
                // Clip level.
                let v = self.ramp22(0x35, 2, inc_c);
                let k = &mut self.k;
                k[0x32] = v;
                let c = v + v;
                k[0x88] = c;
                k[0xbe] = 0.25 / c;
                let nc = -c;
                k[0x89] = nc;
                for v in VOICES {
                    k[v.p + LO] = nc;
                }
            }
            if lfo {
                self.or(0x8b, 4);
            }
            if old8b != 0 && self.gi(0x8b) & B.bit != 0 {
                ev[2] = true;
                self.pitch(B, inc_v);
            }
            if ev_b || ev[2] {
                delay[2] = self.color(B);
            }
            if lfo {
                self.or(0x8b, 8);
            }
            if old22 != 0 && self.gi(0x22) & 4 != 0 {
                // Loop gain.
                let v = self.ramp22(0x39, 4, inc_c);
                self.k[0x43] = v;
                self.k[0x36] = v;
            }
            if old8b != 0 && self.gi(0x8b) & E.bit != 0 {
                ev[3] = true;
                self.pitch(E, inc_v);
            }
            if ev_b || ev[3] {
                delay[3] = self.color(E);
            }
            if old22 != 0 && self.gi(0x22) & 8 != 0 {
                // Input gain.
                let v = self.ramp22(0x3d, 8, inc_c);
                self.k[0x3e] = v;
                self.k[0x3a] = v;
            }
            if lfo {
                self.or(0x8b, 0x10);
            }
        }

        // Audio (§3.3): input high-pass, the resonators' inputs and loop filters.
        let k = &mut self.k;
        let gin = k[0x3e];
        let fb = k[0x43];
        let yr = k[0x41] * (in_r * gin - k[0x52] * k[0x40]);
        let base_r = yr - k[0x52];
        let x_e = base_r + k[E.out] * fb;
        let x_d = base_r + k[D.out] * fb;
        let yl = k[0x41] * (in_l * gin - k[0x40] * k[0x3f]);
        let base_l = yl - k[0x3f];
        let x_b = base_l + k[B.out] * fb;
        let x_a = base_l + k[A.out] * fb;
        let x_f = base_r + k[F.out] * fb;
        let x_c = base_l + k[C.out] * fb;
        let w_e = self.shelf(E, x_e);
        let w_d = self.shelf(D, x_d);
        self.write(E, x_e, w_e, wi);
        self.write(D, x_d, w_d, wi);
        let gate = if self.gi(0x65) > self.gi(0x66) { 1.0 } else { 0.0 };
        let w_b = self.shelf(B, x_b);
        self.write(B, x_b, w_b, wi);
        let w_a = self.shelf(A, x_a);
        self.write(A, x_a, w_a, wi);
        let k = &mut self.k;
        // Level follower (+0x19c): fast up, slow down.
        let d = gate - k[0x67];
        let c = if d > 0.0 { k[0x68] } else { k[0x69] };
        let lv = k[0x67] + (d + DC) * c;
        k[0x67] = lv;
        let lv = lv + -1.0 + 1.0;
        k[0x52] = yr + DC;
        k[0x3f] = yl + DC;
        let w_f = self.shelf(F, x_f);
        let w_c = self.shelf(C, x_c);
        // Wet: the sum of each side's resonator inputs, DC-blocked, two fixed filters.
        let wet_r = self.out_filter(x_f + (x_d + x_e), [0x56, 0x58, 0x57, 0x59, 0x5b, 0x5a, 0x5c]);
        let wet_l = self.out_filter(x_c + (x_a + x_b), [0x46, 0x49, 0x48, 0x4a, 0x4e, 0x4d, 0x4f]);
        let counter = if ctl { 0 } else { self.gi(0x19) + 1 };
        self.si(0x19, counter);
        let g66 = self.gi(0x66) + 1;
        self.si(0x66, g66);
        self.k[0x6a] = lv;
        self.k[0x80] = 0.0;
        let f22_audio = self.gi(0x22);
        self.si(0x22, f22_audio | 0x10);

        // Control block, second part.
        if ctl {
            for (j, v) in [(3, E), (1, D), (0, A), (2, B)] {
                if ev_b || ev[j] {
                    self.taps(v, delay[j]);
                }
            }
            if old8b != 0 && self.gi(0x8b) & C.bit != 0 {
                ev[4] = true;
                self.pitch(C, inc_v);
            }
            if ev_b || ev[4] {
                delay[4] = self.color(C);
            }
            if lfo {
                self.or(0x8b, 0x20);
            }
            if old22 != 0 {
                // Wet level: the level follower × the wet gain, smoothed.
                let mut c = if init { 1.0 } else { inc_c + 0.0 };
                if !(c < 1.0) {
                    self.si(0x22, f22_audio & !0x10);
                    c = 1.0;
                }
                let k = &mut self.k;
                k[0x80] = c;
                let v = k[0x7f] + (lv * k[0x7e] - k[0x7f]) * c;
                k[0x81] = v;
                k[0x7f] = v;
                self.or(0x1b, 2);
                let k = &mut self.k;
                k[0x83] = k[0x82];
                k[0x84] = v * k[0x63] - k[0x82];
                k[0x85] = 0.0;
            }
        }

        // Automatic gain (+0x144): follows the louder wet side.
        let k = &mut self.k;
        let peak = if wet_l.abs() > wet_r.abs() { wet_l.abs() } else { wet_r.abs() };
        let pk = peak * k[0x5d];
        k[0x5e] = pk;
        let lvl = k[0x51];
        let d = (pk + k[0x5f]).abs() - lvl;
        let rate = if d > 0.0 {
            k[0x61] = 1.0;
            k[0x60]
        } else {
            let h = k[0x61];
            k[0x61] = h * k[0x180];
            if h > 0.5 { 0.0 } else { k[0x62] }
        };
        let lvl = lvl + d * rate;
        k[0x51] = lvl + DC;
        let lvl = lvl.abs();
        let gain = if lvl < 0.01 { 100.0 } else { 1.0 / lvl };

        self.write(C, x_c, w_c, wi);
        if ctl {
            if old8b != 0 && self.gi(0x8b) & F.bit != 0 {
                ev[5] = true;
                self.pitch(F, inc_v);
            }
            if ev_b || ev[5] {
                delay[5] = self.color(F);
            }
            if ev_b || ev[4] {
                self.taps(C, delay[4]);
            }
            if old1b != 0 && self.gi(0x1b) & 2 != 0 {
                // Wet gain glide.
                let mut pos = self.k[0x85] + inc_m;
                if !(pos < 1.0) {
                    self.clear(0x1b, 2);
                    pos = 1.0;
                }
                let k = &mut self.k;
                k[0x85] = pos;
                let v = pos * k[0x84] + k[0x83];
                k[0x86] = v;
                k[0x82] = v;
            }
        }
        self.write(F, x_f, w_f, wi);
        let k = &mut self.k;
        k[OUT_L] = in_l * k[0x21] + wet_l * gain * k[0x86];
        k[OUT_R] = in_r * k[0x21] + wet_r * gain * k[0x86];
        if ctl && (ev_b || ev[5]) {
            self.taps(F, delay[5]);
        }
        for v in VOICES {
            self.read(v, w);
        }
    }

    /// The wet path of one side: DC blocker, then two one-pole sections mixed back in.
    /// `c`: DC state, section 1 state, g − 1, 1/(g + 1), section 2 state, g − 1, 1/(g + 1).
    fn out_filter(&mut self, sum: f32, c: [usize; 7]) -> f32 {
        let k = &mut self.k;
        let m = k[c[0]] + (sum - k[c[0]]) * k[0x47];
        let mut x = sum - m;
        let s = k[c[1]];
        let y = k[c[3]] * (x - s * k[c[2]]);
        k[c[1]] = y + DC;
        x += k[0x4b] * (y - s);
        let s = k[c[4]];
        let y = k[c[6]] * (x - s * k[c[5]]);
        k[c[4]] = y + DC;
        x += k[0x50] * (k[0x4c] * (s + y));
        k[c[0]] = m + DC;
        x
    }
    /// Re-aim every voice's pitch glide at its chord value + vibrato + tune + spread.
    fn aim(&mut self, v: Voice) {
        let t = self.k[v.ns] + self.k[v.cl];
        self.glide(v, t);
    }

    /// `processEventInSpread` (§3.1).
    fn ev_spread(&mut self) {
        let s2 = self.k[SPREAD] * self.k[SPREAD];
        self.k[0x75] = s2;
        let sp = if self.gi(0x78) < 1 { s2 } else { 0.0 };
        for v in [F, C, E, B, D, A] {
            let k = &mut self.k;
            k[v.sp] = sp * v.spread;
            k[v.ns] = k[0x9b] + sp * v.spread;
            self.aim(v);
        }
        self.or(0x8b, 0x3f);
        let k = &mut self.k;
        let m = 1.0 - s2;
        let d = if m > k[0x76] {
            let x = (m - k[0x76]) * k[0x77];
            k[0x79] * (x * x)
        } else {
            0.0
        };
        k[0x7a] = 1.0 - d;
        k[0x7b] = k[0x7a] + k[0x74];
        k[0x7d] = k[0x7b] + k[0x7c];
        k[0x7e] = k[0x7d] * k[0x71];
        k[0x80] = 0.0;
        self.or(0x22, 0x10);
    }

    /// `processEventInFeedback`: input gain, loop gain and clip level from three tables,
    /// the follower's attack, and the wet-level trim above 50 %.
    fn ev_feedback(&mut self) {
        let k = &mut self.k;
        let v = k[FEEDBACK];
        let a = if v > k[0x6e] { (v - k[0x6e]) * k[0x6f] * 0.3 } else { 0.0 };
        let h = v + v;
        let q = if h > 1.0 { h + -1.0 } else { 0.0 };
        let p = v * 40.0;
        let n = rnd(p + -0.5);
        let i0 = if (n as u32) < 40 { n as usize } else { 40 };
        let f = p - n as f32;
        let g = 1.0 - f;
        let m = rnd(n as f32 + 1.0);
        let i1 = if (m as u32) < 40 { m as usize } else { 40 };
        let lerp = |t: &[f32; 41]| g * t[i0] + f * t[i1];
        k[0x3b] = k[0x3a];
        k[0x3c] = lerp(&IN_GAIN) * T3 - k[0x3a];
        k[0x3d] = 0.0;
        k[0x37] = k[0x36];
        k[0x38] = lerp(&FB_GAIN) - k[0x36];
        k[0x39] = 0.0;
        k[0x33] = k[0x32];
        k[0x34] = lerp(&CLIP) * 0.5 - k[0x32];
        k[0x35] = 0.0;
        let q = q * (2.0 - q) * 0.99 + -1.0 + 1.0;
        let q = q * q;
        k[0x5d] = q;
        k[0x5f] = q * -0.75 + 1.0;
        k[0x70] = 1.0 - a;
        k[0x71] = (1.0 - a) * k[0x6d];
        k[0x7e] = k[0x71] * k[0x7d];
        k[0x80] = 0.0;
        self.or(0x22, 0x1e);
    }

    /// `processEventInDecay`: the envelope's hold time, threshold and release.
    fn ev_decay(&mut self) {
        let k = &mut self.k;
        let h = k[DECAY] + k[DECAY];
        let (a, b) = if h > 1.0 { (h + -1.0, 1.0) } else { (0.0, h) };
        let a3 = a * (a * a) * 0.9;
        let b3 = b * (b * b) * 0.1;
        let s = a3 + b3;
        let dd = s * s;
        let hold = rnd(k[0x25] * (488.0 * dd + 12.0));
        k[0x24] = -0.19 * dd + 0.2;
        let m = 1.0 - s;
        let r = if m > k[0x72] {
            let x = (m - k[0x72]) * k[0x73];
            x * x * 0.3
        } else {
            0.0
        };
        let t = (k[0x2c] * (dd * 7992.0 + 8.0) + -1.0).max(1.0);
        k[0x2d] = 1.0 - exp2_tab(k[0x2b] / t);
        k[0x74] = r;
        k[0x7b] = r + k[0x7a];
        k[0x7d] = k[0x7b] + k[0x7c];
        k[0x7e] = k[0x7d] * k[0x71];
        k[0x80] = 0.0;
        self.si(0x26, hold);
        self.or(0x22, 0x10);
    }

    /// `processEventInMode`: String (0) or Chord (1).
    fn ev_mode(&mut self) {
        let m = rnd(self.k[MODE]);
        self.si(0x78, m);
        let (sp, x79, base, mult, x7c, depth) =
            if m < 1 { (self.k[0x75], 0.25, 36, 0.0, -0.2, 0.0) } else { (0.0, 0.0, 60, 1.0, 0.0, 0.07) };
        let k = &mut self.k;
        k[0x7c] = x7c;
        k[0x95] = depth;
        k[0x79] = x79;
        self.si(0x9a, base);
        let nb = (self.gi(0x99) + base) as f32;
        let k = &mut self.k;
        k[0x9b] = nb;
        k[0x90] = mult;
        for v in VOICES {
            let k = &mut self.k;
            k[v.sp] = sp * v.spread;
            k[v.ns] = sp * v.spread + nb;
            k[v.cv] = mult * k[v.tab];
            k[v.cl] = k[v.cv] + k[v.lfo];
            self.aim(v);
        }
        let k = &mut self.k;
        k[0x7d] = k[0x7c] + k[0x7b];
        k[0x7e] = k[0x7d] * k[0x71];
        k[0x80] = 0.0;
        self.or(0x22, 0x10);
        self.or(0x8b, 0x3f);
    }

    /// `processEventInStyle` / `processEventInChord`: look the six voices up in the table.
    fn ev_chord(&mut self) {
        let idx = self.gi(0x8d) + self.gi(0x8e);
        for (j, v) in VOICES.into_iter().enumerate() {
            let i = idx.wrapping_add(j as i32) as u32;
            let k = &mut self.k;
            k[v.tab] = CHORDS[if i > 0x94 { 0x95 } else { i as usize }];
            k[v.cv] = k[v.tab] * k[0x90];
            k[v.cl] = k[v.cv] + k[v.lfo];
            self.aim(v);
        }
        self.or(0x8b, 0x3f);
    }

    fn ev_style(&mut self) {
        let s = rnd(rnd(self.k[STYLE]) as f32);
        self.si(0x8d, s * 30);
        self.ev_chord();
    }

    fn ev_chord_sel(&mut self) {
        let c = rnd(rnd(self.k[CHORD]) as f32);
        self.si(0x8e, c * 6);
        self.ev_chord();
    }

    /// `processIntParameter` (Tune, semitones).
    fn ev_tune(&mut self, t: i32) {
        self.k[TUNE] = t as f32;
        let n = rnd(t as f32);
        self.si(0x99, n);
        let nb = (self.gi(0x9a) + n) as f32;
        self.k[0x9b] = nb;
        for v in [F, C, E, B, D, A] {
            self.k[v.ns] = self.k[v.sp] + nb;
            self.aim(v);
        }
        self.or(0x8b, 0x3f);
    }

    /// `processFloatParameter` for Brightness (inline in the processor).
    fn ev_brightness(&mut self) {
        let k = &mut self.k;
        let v = k[BRIGHTNESS];
        k[0x2f] = (v.abs() * -0.6 + 1.6) * v;
        let a = if k[0x6b] < v { (v - k[0x6b]) * k[0x6c] * 0.33 } else { 0.0 };
        k[0x6d] = 1.0 - a;
        k[0x71] = k[0x6d] * k[0x70];
        k[0x7e] = k[0x71] * k[0x7d];
        k[0x80] = 0.0;
        k[0x31] = 0.0;
        self.or(0x22, 0x11);
    }

    /// `processFloatParameter` for Mix (inline): dry and wet glides.
    fn ev_mix(&mut self) {
        let k = &mut self.k;
        let v = k[MIX];
        k[0x1e] = k[0x1d];
        k[0x1f] = (1.0 - v) - k[0x1d];
        k[0x20] = 0.0;
        let wet = (2.0 - v) * v;
        k[0x63] = wet;
        let t = wet * k[0x81];
        k[0x83] = k[0x82];
        k[0x84] = t - k[0x82];
        k[0x85] = 0.0;
        self.or(0x1b, 3);
    }

    /// `resume()` + `endInit()`: clear everything, set the rate-dependent constants, take
    /// every parameter, and run one silent sample with all glides completed (§2).
    fn resume(&mut self, sr: f32, inputs: &[(usize, f32)], tune: i32) {
        self.k.fill(0.0);
        let k = &mut self.k;
        k[0] = sr;
        for &(i, v) in inputs {
            k[i] = v;
        }
        // log2(0.1) from the kernel's log table (entries 612/613, interpolated).
        let l =
            f32::from_bits(0x3f2d_7364) * f32::from_bits(0x3ecc_c800) + f32::from_bits(0x3f2d_ad43) * f32::from_bits(0x3f19_9c00) + -4.0;
        k[0x2b] = l;
        k[0x4b] = f32::from_bits(0x3ed3_4570); // 0.412639
        k[0x50] = f32::from_bits(0x3ed7_3964); // 0.420360
        let steps = rnd(sr / 8820.0);
        let ctl = if steps == 0 { 0 } else { rnd(sr) / steps };
        let ctl_f = ctl as f32;
        let ctl50 = (rnd(ctl_f) / 50) as f32;
        let pisr = PI / sr;
        for v in VOICES {
            k[v.p + SHELF] = f32::from_bits(0xbca2_2600); // −0.0197935
            k[v.p + MAXF] = sr * 0.9;
            k[v.p + PISR] = pisr;
            k[v.p + MAXF2] = sr * 0.494;
            k[v.p + PISR2] = pisr;
        }
        k[0x93] = 1.0 / ctl50;
        let ramp = 1.0 / (ctl_f * 0.02);
        k[0x8c] = ramp;
        k[0x1c] = ramp;
        k[0x23] = ramp;
        let dc = TWO_PI / sr * 30.0;
        k[0x47] = if dc > 1.0 { 1.0 } else { dc };
        let clamp = |c: f32| {
            let f = if sr > c { c } else { sr };
            if f < 0.0 { 0.0 } else { f }
        };
        let (f1, f2, f3) = (
            clamp(f32::from_bits(0x459b_ff1d)), // 4991.889 Hz
            clamp(f32::from_bits(0x4338_98b6)), // 184.5965 Hz
            clamp(f32::from_bits(0x4335_ceed)), // 181.8083 Hz
        );
        let c1k = ctl_f / 1000.0;
        let s1k = sr / 1000.0;
        let hold = s1k * 1000.0;
        k[0x2c] = c1k;
        k[0x25] = c1k;
        let a = sr * 0.01;
        k[0x180] = if a < 1.0 { f32::from_bits(0x3e9d_1bd6) } else { 1.0 - LN2 / a };
        let a = sr * 0.05;
        k[0x62] = if a < 1.0 { LN2 } else { LN2 / a };
        let a = sr * 0.001;
        k[0x60] = if a < 1.0 { LN2 } else { LN2 / a };
        let one = (s1k * 0.0 + -1.0).max(1.0);
        k[0x68] = 1.0 - exp2_tab(l / one);
        k[0x69] = 1.0 - exp2_tab(l / (hold + -1.0).max(1.0));
        k[0x2a] = 1.0 - exp2_tab(l / (c1k * 15.0 + -1.0).max(1.0));
        for (i, v) in [(0x6b, 0.5), (0x6c, 2.0), (0x6e, 0.5), (0x6f, 2.0), (0x72, 0.5)] {
            k[i] = v;
        }
        k[0x73] = 2.0;
        k[0x76] = 0.75;
        k[0x77] = 4.0;
        let g = tan_approx(pisr * f1);
        k[0x59] = 1.0 / (g + 1.0);
        k[0x4a] = k[0x59];
        k[0x57] = g + -1.0;
        k[0x48] = k[0x57];
        let g = tan_approx(pisr * f2);
        k[0x4c] = g;
        k[0x5c] = 1.0 / (g + 1.0);
        k[0x4f] = k[0x5c];
        k[0x5a] = g + -1.0;
        k[0x4d] = k[0x5a];
        let g = tan_approx(pisr * f3);
        for v in VOICES {
            k[v.p + G] = g;
            k[v.p + INV] = 1.0 / (g + 1.0);
            k[v.p + GM1] = g + -1.0;
        }
        let g = tan_approx(pisr * (sr * 0.494).min(30.0));
        k[0x40] = g + -1.0;
        k[0x41] = 1.0 / (g + 1.0);
        self.si(0x64, rnd(hold));
        self.si(0x1a, steps - 1);
        self.si(0x19, steps);
        self.si(0x92, 50);

        // The parameters, in an order where each sees the cells it reads (§2).
        self.ev_spread();
        self.ev_mode();
        self.ev_tune(tune);
        self.ev_style();
        self.ev_chord_sel();
        self.ev_spread();
        self.ev_decay();
        self.ev_feedback();
        self.ev_brightness();
        self.ev_mix();
        self.tick(true);
    }
}

/// Maschine's Resochord.
pub struct Resochord {
    core: Core,
    sr: f32,
    /// The parameters last taken (`None` before the first `set`).
    last: Option<[f64; 9]>,
}

impl Resochord {
    pub fn new(sr: f64) -> Self {
        Resochord { core: Core::new(), sr: sr as f32, last: None }
    }
}

/// Fields in `set` order, with the kernel input each one drives and its default.
const FIELDS: [(&str, usize, f64); 9] = [
    ("mode", MODE, 1.0),
    ("style", STYLE, 0.0),
    ("chord", CHORD, 1.0),
    ("tune", TUNE, 0.0),
    ("spread", SPREAD, 0.5),
    ("brightness", BRIGHTNESS, 0.75),
    ("feedback", FEEDBACK, 0.75),
    ("decay", DECAY, 0.5),
    ("mix", MIX, 0.75),
];

impl Effect for Resochord {
    fn set(&mut self, params: &[(String, f64)]) {
        let vals = FIELDS.map(|(f, _, d)| param(params, f, d));
        let tune = vals[3] as i32;
        let c = &mut self.core;
        match self.last {
            None => {
                // The processor resumes with the stored values.
                let inputs: Vec<(usize, f32)> = FIELDS.iter().zip(vals).map(|(&(_, i, _), v)| (i, v as f32)).collect();
                c.resume(self.sr, &inputs, tune);
            }
            Some(old) => {
                // Only changed parameters send events (the glides and filters go on).
                for (j, (&(_, i, _), v)) in FIELDS.iter().zip(vals).enumerate() {
                    if v == old[j] {
                        continue;
                    }
                    if i == TUNE {
                        c.ev_tune(tune);
                        continue;
                    }
                    c.k[i] = v as f32;
                    match i {
                        MODE => c.ev_mode(),
                        STYLE => c.ev_style(),
                        CHORD => c.ev_chord_sel(),
                        SPREAD => c.ev_spread(),
                        BRIGHTNESS => c.ev_brightness(),
                        FEEDBACK => c.ev_feedback(),
                        DECAY => c.ev_decay(),
                        _ => c.ev_mix(),
                    }
                }
            }
        }
        self.last = Some(vals);
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let c = &mut self.core;
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            c.k[IN_L] = *a;
            c.k[IN_R] = *b;
            c.tick(false);
            *a = c.k[OUT_L];
            *b = c.k[OUT_R];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pairs: &[(&str, f64)]) -> Vec<(String, f64)> {
        pairs.iter().map(|&(k, v)| (k.to_string(), v)).collect()
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1u32 << 24) as f32 * 0.5 - 0.25
            })
            .collect()
    }

    fn run(fx: &mut Resochord, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (l.to_vec(), r.to_vec());
        for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            fx.process(a, b);
        }
        (l, r)
    }

    #[test]
    fn constants_are_the_kernels() {
        for (v, bits) in [
            (OCT, 0x3daa_a880),
            (NOTE0, 0x4102_d6b6),
            (T3, 0x3eaa_aa9f),
            (T5, 0x3e08_8872),
            (ATAN[0], 0x3b3b_d75c),
            (ATAN[1], 0xbc84_6dee),
            (ATAN[2], 0x3d2f_c1fa),
            (ATAN[3], 0xbd9a_316f),
            (ATAN[4], 0x3dda_3db4),
            (ATAN[5], 0xbe11_7fc7),
            (ATAN[6], 0x3e4c_bc06),
            (ATAN[7], 0xbeaa_aa5c),
            (0.666, 0x3f2a_7efa),
            (0.444, 0x3ee3_53f8),
            (0.222, 0x3e63_53f8),
            (0.056, 0x3d65_6042),
            (0.242, 0x3e77_ced9),
            (0.693, 0x3f31_6873),
            (0.248, 0x3e7d_f3b6),
            (0.708, 0x3f35_3f7d),
            (0.494, 0x3efc_ed91),
            (0.99, 0x3f7d_70a4),
            (-0.19, 0xbe42_8f5c),
        ] {
            assert_eq!(v.to_bits(), bits, "{v}");
        }
        assert_eq!(CHORDS[3].to_bits(), 0x407b_645a); // 3.928
    }

    #[test]
    fn exp2_tab_is_two_to_the_x() {
        for x in [-3.3f32, -1.0, -0.25, 0.0, 0.4, 2.7] {
            let e = exp2_tab(x);
            assert!((e as f64 / (x as f64).exp2() - 1.0).abs() < 2e-6, "{x}: {e}");
        }
    }

    #[test]
    fn defaults_resonate_and_stay_finite() {
        let x = noise(88_200, 1);
        let mut fx = Resochord::new(44_100.0);
        fx.set(&[]);
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        assert!(l.iter().zip(&x).any(|(a, b)| a != b));
        // Silence afterwards: the resonators ring out without blowing up.
        let z = vec![0.0; 441_000];
        let (l, r) = run(&mut fx, &z, &z);
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 4.0));
    }

    #[test]
    fn extremes_stay_finite() {
        let x: Vec<f32> = noise(44_100, 2).iter().map(|v| v * 4.0).collect();
        for mode in [0.0, 1.0] {
            for (fb, decay, br) in [(1.0, 1.0, 1.0), (0.0, 0.0, 0.0), (1.0, 0.0, 1.0)] {
                let mut fx = Resochord::new(48_000.0);
                fx.set(&p(&[
                    ("mode", mode),
                    ("feedback", fb),
                    ("decay", decay),
                    ("brightness", br),
                    ("tune", 24.0),
                    ("spread", 1.0),
                    ("mix", 1.0),
                ]));
                let (l, r) = run(&mut fx, &x, &x);
                assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 100.0));
            }
        }
    }

    #[test]
    fn mix_zero_is_the_input() {
        let x = noise(20_000, 3);
        let y = noise(20_000, 4);
        let mut fx = Resochord::new(44_100.0);
        fx.set(&p(&[("mix", 0.0)]));
        let (l, r) = run(&mut fx, &x, &y);
        assert_eq!(l, x);
        assert_eq!(r, y);
    }

    #[test]
    fn right_side_quiet_for_left_only_input() {
        let x = noise(20_000, 5);
        let z = vec![0.0; 20_000];
        let mut fx = Resochord::new(44_100.0);
        fx.set(&p(&[("mix", 1.0)]));
        let (l, r) = run(&mut fx, &x, &z);
        // Only the kernel's denormal guards (1e-18 per state) reach the right side.
        assert!(r.iter().all(|&v| v.abs() < 1e-12));
        assert!(l.iter().any(|&v| v.abs() > 0.01));
    }

    #[test]
    fn set_keeps_state() {
        let x = noise(40_000, 6);
        let params = p(&[("mode", 0.0), ("tune", -5.0), ("mix", 0.6)]);
        let mut a = Resochord::new(44_100.0);
        a.set(&params);
        let (l1, _) = run(&mut a, &x[..10_000], &x[..10_000]);
        // Re-sending the same values changes nothing.
        a.set(&params);
        let (l2, _) = run(&mut a, &x[10_000..], &x[10_000..]);
        let mut b = Resochord::new(44_100.0);
        b.set(&params);
        let (lb, _) = run(&mut b, &x, &x);
        assert_eq!([l1, l2].concat(), lb);
        // A change glides: the output stays continuous and finite.
        a.set(&p(&[("mode", 1.0), ("style", 3.0), ("chord", 4.0), ("tune", 12.0), ("mix", 1.0)]));
        let (l3, _) = run(&mut a, &x, &x);
        assert!(l3.iter().all(|v| v.is_finite()));
    }
}
