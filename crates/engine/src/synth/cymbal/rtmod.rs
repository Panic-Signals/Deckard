//! `NI::DSP::RealTimeModulation` and the cymbal's two subclasses
//! (`UTIL::CrashCymbalRealTimeMod`, `UTIL::RideCymbalRealTimeMod`): the macro layer
//! between the panel's knobs and the DSP (docs/synth-cymbal.md §4).
//!
//! Each knob value (and velocity) is clamped, shaped, and stored as one of eight macro
//! inputs `x_k` (object +0x08 + 4k); a *target* `t` (a DSP setting the core reads from
//! `out[t]`) is a sum of lines `lo_kt + x_k·(hi_kt − lo_kt)` over the macros that drive
//! it, plus a velocity line `lo + w·(hi − lo)` where `w` is velocity through one of five
//! curves. The cores' `initPreset` fills the tables. Which macros and which curve each
//! target sums, and in which order, is written out per knob in Maschine; the functions
//! here follow that order exactly (each line one fused multiply-add, each sum a left
//! fold).
//!
//! The object's scalar fields are mirrored by offset: [`RtMod::f`] holds +0x00..+0x94
//! (index = offset/4).

#![allow(dead_code)]

/// One line of a macro table (`sModulationValues`, 12 bytes): (flag, lo, hi).
pub(super) type Line = [f32; 3];
/// One line of the velocity table (`sVelocityValues`, 16 bytes): (flag, curve, lo, hi).
pub(super) type VelLine = [f32; 4];

const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}

#[derive(Clone, Debug)]
pub(super) struct RtMod {
    /// +0x00..+0x94 by offset/4: +0x08 + 4k the macro inputs `x_k`; +0x28..+0x40
    /// shaped values the cores read directly; +0x44/+0x58/+0x6c/+0x80 + 4c the five
    /// velocity curves (low, high, shape, mix).
    pub f: [f32; 38],
    /// +0x98: the raw values (`[0]` velocity, then the knobs).
    pub values: Vec<f32>,
    /// +0xb0: the targets.
    pub out: Vec<f32>,
    /// +0xc8: the velocity table.
    pub vel: Vec<VelLine>,
    /// +0xe0 + 0x18k: the macro tables.
    pub tab: [Vec<Line>; 8],
}

#[inline]
fn clamp01(x: f32) -> f32 {
    let x = if x < 0.0 { 0.0 } else { x };
    if x > 1.0 { 1.0 } else { x }
}

/// `lo + x·(hi − lo)`, fused.
#[inline]
fn lerp(lo: f32, hi: f32, x: f32) -> f32 {
    (hi - lo).mul_add(x, lo)
}

/// Maschine's curve tail `a = 1 − x; a += 1 − a; x = a + (x − a)` (constant-folded
/// blend; its rounding matters).
#[inline]
fn fold(x: f32) -> f32 {
    let a = 1.0 - x;
    let a = a + (1.0 - a);
    a + (x - a)
}

impl RtMod {
    /// `RealTimeModulation(macros, targets)` (0x1ccce48).
    pub fn new(macros: usize, targets: usize) -> RtMod {
        RtMod {
            f: [0.0; 38],
            values: vec![0.0; macros],
            out: vec![0.0; targets],
            vel: vec![[0.0; 4]; targets],
            tab: std::array::from_fn(|_| vec![[0.0; 3]; targets]),
        }
    }

    #[inline]
    pub fn get(&self, off: usize) -> f32 {
        self.f[off / 4]
    }
    #[inline]
    pub fn put(&mut self, off: usize, v: f32) {
        self.f[off / 4] = v;
    }

    /// Velocity (`values[0]`) through the curve that velocity line `r` names.
    pub fn curve(&self, r: usize) -> f32 {
        let i = self.vel[r][1] as i32 as usize;
        let (lo, hi0, shape, mix) = (self.f[0x44 / 4 + i], self.f[0x58 / 4 + i], self.f[0x6c / 4 + i], self.f[0x80 / 4 + i]);
        let hi = if hi0 > lo { hi0 } else { lo + c(0x3b03_126f) };
        let v = self.values[0];
        let v = if v < lo { lo } else { v };
        let v = if v > hi { hi } else { v };
        let d = hi - lo;
        let d = if d < c(0x33d6_bf95) { c(0x33d6_bf95) } else { d };
        let x = (v - lo) / d;
        let x = x * (-x.abs()).mul_add(shape, shape + 1.0);
        let a = 1.0 - x;
        let m = if mix + mix > 1.0 { 1.0 } else { mix + mix };
        let a = (1.0 - a).mul_add(m, a);
        let k = mix.mul_add(2.0, -m);
        clamp01((x - a).mul_add(k, a))
    }

    /// Velocity line `t` at curve value `w`.
    #[inline]
    pub fn v(&self, t: usize, w: f32) -> f32 {
        lerp(self.vel[t][2], self.vel[t][3], w)
    }

    /// Macro `k`'s line for target `t`, at its input `x_k`.
    #[inline]
    pub fn m(&self, k: usize, t: usize) -> f32 {
        lerp(self.tab[k][t][1], self.tab[k][t][2], self.f[2 + k])
    }

    /// Macro `k`'s line for target `t` at `x`.
    #[inline]
    pub fn mx(&self, k: usize, t: usize, x: f32) -> f32 {
        lerp(self.tab[k][t][1], self.tab[k][t][2], x)
    }

    // --- Crash (`CrashCymbalRealTimeMod`, 8 macros, 18 targets): macros 0 tune,
    // 1 decay, 2 density, 3 tone, 4 a density-derived input; values [1] tune,
    // [2] decay, [3] density, [4] tone. ---

    /// `CrashCymbalRealTimeMod::setTune` (0x1cd186c).
    pub fn crash_tune(&mut self, v: f32) {
        let t = clamp01(v);
        self.values[1] = t;
        self.put(0x08, t);
        let t2 = t + t;
        self.put(0x20, clamp01(t2));
        self.put(0x24, t);
        let a = if t > 0.5 { 1.0 } else { t2 };
        let a = a * a.abs().mul_add(c(0xbecc_cccd), c(0x3fb3_3333));
        let a = clamp01(fold(a));
        self.put(0x28, a.mul_add(c(0xbe0d_4f16), c(0x3e0d_4f16)));
        let k = t.abs();
        let p = fold((-c(0x3e4c_cccd)).mul_add(k, c(0x3f99_999a)) * t);
        let q = fold(1.0f32.mul_add(k, 0.0) * t);
        self.put(0x34, clamp01(q).mul_add(c(0x40ad_708f), c(0xc05a_e11e)));
        self.put(0x3c, clamp01(p).mul_add(c(0x3d03_1401), c(0xbd5d_2bf5)));
        let w = self.curve(1);
        self.out[7] = ((self.mx(0, 7, t) + self.v(7, w)) + self.m(1, 7)) + self.m(2, 7);
        self.out[8] = ((self.v(8, w) + self.m(0, 8)) + self.m(1, 8)) + self.m(2, 8);
        self.out[9] = self.m(0, 9);
        self.out[10] = ((self.v(10, w) + self.m(0, 10)) + self.m(1, 10)) + self.m(2, 10);
        self.out[16] = self.m(0, 16);
    }

    /// `CrashCymbalRealTimeMod::setDecay` (0x1cd1b84).
    pub fn crash_decay(&mut self, v: f32) {
        let d = clamp01(v);
        self.values[2] = d;
        self.put(0x0c, d);
        let a = d * d.abs().mul_add(c(0xbecc_cccd), c(0x3fb3_3333));
        let a = clamp01(fold(a));
        self.put(0x38, a.mul_add(c(0xbe9b_a7fc), c(0x3f01_06bc)));
        let w = self.curve(1);
        self.out[0] = self.mx(1, 0, d) + self.v(0, w);
        self.out[1] = self.m(1, 1) + self.v(1, w);
        self.out[7] = ((self.v(7, w) + self.m(0, 7)) + self.m(1, 7)) + self.m(2, 7);
        self.out[8] = ((self.v(8, w) + self.m(0, 8)) + self.m(1, 8)) + self.m(2, 8);
        self.out[10] = ((self.v(10, w) + self.m(0, 10)) + self.m(1, 10)) + self.m(2, 10);
        self.out[13] = self.m(1, 13);
        self.out[17] = (self.v(17, w) + self.m(1, 17)) + self.m(2, 17);
    }

    /// `CrashCymbalRealTimeMod::setDensity` (0x1cd1e40).
    pub fn crash_density(&mut self, v: f32) {
        let d = clamp01(v);
        self.values[3] = d;
        let a = d * d.abs().mul_add(c(0xbf4c_cccd), c(0x3fe6_6666));
        self.put(0x10, clamp01(fold(a)));
        let e = clamp01(self.values[3]);
        let e = clamp01(fold(e * (2.0 - e.abs())));
        self.put(0x2c, e.mul_add(c(0x3f67_6d80), c(0x3dc4_93fe)));
        let w2 = self.curve(2);
        self.out[2] = (self.m(3, 2) + self.v(2, w2)) + self.m(2, 2);
        let w = self.curve(1);
        self.out[7] = ((self.v(7, w) + self.m(0, 7)) + self.m(1, 7)) + self.m(2, 7);
        self.out[8] = ((self.v(8, w) + self.m(0, 8)) + self.m(1, 8)) + self.m(2, 8);
        self.out[10] = ((self.v(10, w) + self.m(0, 10)) + self.m(1, 10)) + self.m(2, 10);
        self.out[17] = (self.v(17, w) + self.m(1, 17)) + self.m(2, 17);
        let w4 = self.curve(4);
        self.out[11] = (self.m(3, 11) + self.v(11, w4)) + self.m(2, 11);
        self.out[15] = self.m(2, 15) + self.m(4, 15);
    }

    /// `CrashCymbalRealTimeMod::setTone` (0x1cd22b8).
    pub fn crash_tone(&mut self, v: f32) {
        let t = clamp01(v);
        self.values[4] = t;
        let a = clamp01(fold(t * (2.0 - t.abs())));
        self.put(0x14, a);
        let e = clamp01(self.values[4]);
        let e = clamp01(fold(e * (2.0 - e.abs())));
        self.put(0x40, e.mul_add(c(0xbedc_28f6), c(0xbe75_c209)));
        let w2 = self.curve(2);
        self.out[2] = (self.mx(3, 2, a) + self.v(2, w2)) + self.m(2, 2);
        self.out[6] = self.m(3, 6);
        let w4 = self.curve(4);
        self.out[11] = (self.m(3, 11) + self.v(11, w4)) + self.m(2, 11);
        self.out[14] = self.m(3, 14);
    }

    /// `CrashCymbalRealTimeMod::computePerformance` (0x1cd2568): the velocity-driven
    /// targets, at note-on.
    pub fn crash_performance(&mut self) {
        let w = self.curve(1);
        self.out[0] = self.m(1, 0) + self.v(0, w);
        self.out[1] = self.m(1, 1) + self.v(1, w);
        self.out[3] = self.v(3, w);
        self.out[4] = self.v(4, w);
        self.out[5] = self.v(5, w);
        let w2 = self.curve(2);
        self.out[2] = (self.m(3, 2) + self.v(2, w2)) + self.m(2, 2);
        self.out[17] = (self.v(17, w2) + self.m(1, 17)) + self.m(2, 17);
        let w4 = self.curve(4);
        self.out[11] = (self.m(3, 11) + self.v(11, w4)) + self.m(2, 11);
        self.out[12] = self.v(12, w4);
    }

    // --- Ride (`RideCymbalRealTimeMod`, 9 macros, 17 targets): macros 0 tune,
    // 1 decay, 2 edge, 3 bell, 4 impact, 5 a further input; values [1] tune, [2] decay,
    // [3] edge, [4] bell, [5] impact. ---

    /// `RideCymbalRealTimeMod::setTune` (0x1cd289c).
    pub fn ride_tune(&mut self, v: f32) {
        let t = clamp01(v);
        self.values[1] = t;
        let a = clamp01(fold(t * t.abs().mul_add(c(0x3ecc_cccd), c(0x3f19_999a))));
        self.put(0x08, a);
        self.put(0x20, self.values[1]);
        let e = clamp01(self.values[1]);
        let e = clamp01(fold(e * (2.0 - e.abs())));
        self.put(0x34, e.mul_add(c(0x419e_6666), c(0x3ecc_cd10)));
        let w3 = self.curve(3);
        self.out[2] = (self.mx(0, 2, a) + self.v(2, w3)) + self.m(4, 2);
        self.out[3] = (self.v(3, w3) + self.m(0, 3)) + self.m(4, 3);
        self.out[8] = (((self.v(8, w3) + self.m(0, 8)) + self.m(1, 8)) + self.m(2, 8)) + self.m(3, 8);
        let w1 = self.curve(1);
        self.out[6] = (self.v(6, w1) + self.m(0, 6)) + self.m(1, 6);
        self.out[9] = self.m(0, 9);
        self.out[15] = self.m(0, 15);
    }

    /// `RideCymbalRealTimeMod::setDecay`.
    pub fn ride_decay(&mut self, v: f32) {
        let d = clamp01(v);
        self.values[2] = d;
        let x = (d + c(0xbbc4_9ba6)) / c(0x3f7e_76c9);
        let x = if d < c(0x3bc4_9ba6) { 0.0 } else { x };
        let x = clamp01(fold(x * (2.0 - x.abs())));
        self.put(0x0c, x);
        let w1 = self.curve(1);
        self.out[6] = (self.m(0, 6) + self.v(6, w1)) + self.mx(1, 6, x);
        let w3 = self.curve(3);
        self.out[8] = (((self.m(0, 8) + self.v(8, w3)) + self.m(1, 8)) + self.m(2, 8)) + self.m(3, 8);
        self.out[10] = self.m(1, 10);
        self.out[12] = self.m(1, 12);
        let w4 = self.curve(4);
        self.out[16] = self.m(1, 16) + self.v(16, w4);
    }

    /// `RideCymbalRealTimeMod::setEdge`.
    pub fn ride_edge(&mut self, v: f32) {
        let e = clamp01(v);
        self.values[3] = e;
        self.put(0x10, e);
        let x = clamp01(self.values[3]);
        self.put(0x2c, clamp01(fold(x * (2.0 - x.abs()))));
        self.out[7] = self.mx(2, 7, e);
        let w3 = self.curve(3);
        self.out[8] = (((self.v(8, w3) + self.m(0, 8)) + self.m(1, 8)) + self.m(2, 8)) + self.m(3, 8);
    }

    /// `RideCymbalRealTimeMod::setBell`.
    pub fn ride_bell(&mut self, v: f32) {
        let b = clamp01(v);
        self.values[4] = b;
        self.put(0x14, b);
        let x = clamp01(self.values[4]);
        let x = clamp01(fold(x * (x.abs() + 0.0)));
        self.put(0x28, x.mul_add(c(0x3f4b_c686), 0.0));
        let w3 = self.curve(3);
        self.out[8] = (((self.v(8, w3) + self.m(0, 8)) + self.m(1, 8)) + self.m(2, 8)) + self.mx(3, 8, b);
    }

    /// `RideCymbalRealTimeMod::setImpact`.
    pub fn ride_impact(&mut self, v: f32) {
        let i = clamp01(v);
        self.values[5] = i;
        let x = clamp01(fold(i * (2.0 - i.abs())));
        self.put(0x18, x);
        let w3 = self.curve(3);
        self.out[1] = self.mx(4, 1, x) + self.v(1, w3);
        self.out[2] = (self.v(2, w3) + self.m(0, 2)) + self.m(4, 2);
        self.out[3] = (self.v(3, w3) + self.m(0, 3)) + self.m(4, 3);
        self.out[11] = self.m(4, 11);
        self.out[14] = self.m(4, 14) + self.m(5, 14);
    }

    /// `RideCymbalRealTimeMod::computePerformance`.
    pub fn ride_performance(&mut self) {
        let w1 = self.curve(1);
        self.out[0] = self.v(0, w1);
        self.out[4] = self.v(4, w1);
        let w4 = self.curve(4);
        self.out[5] = self.v(5, w4);
        let w3 = self.curve(3);
        self.out[13] = self.v(13, w3);
        let w4 = self.curve(4);
        self.out[16] = self.m(1, 16) + self.v(16, w4);
    }
}
