//! Shared pieces of the Bass Synth's Reaktor-Core kernels (`NI::DSP::UTIL::RX::*`): a
//! word-addressed state block laid out like the compiled kernel's (so each kernel reads
//! as the decompiled code does, offset for offset), and the kernels' lookup tables.

/// A kernel's state: `N` 32-bit words, addressed by byte offset as in the original
/// (`f32`, `i32` and 8-byte-aligned `f64` fields side by side).
#[derive(Clone)]
pub(crate) struct Mem<const N: usize>(pub(crate) [u32; N]);

impl<const N: usize> Mem<N> {
    pub(crate) const fn new() -> Mem<N> {
        Mem([0; N])
    }
    #[inline(always)]
    pub(crate) fn f(&self, off: usize) -> f32 {
        f32::from_bits(self.0[off >> 2])
    }
    #[inline(always)]
    pub(crate) fn set_f(&mut self, off: usize, v: f32) {
        self.0[off >> 2] = v.to_bits();
    }
    #[inline(always)]
    pub(crate) fn i(&self, off: usize) -> i32 {
        self.0[off >> 2] as i32
    }
    #[inline(always)]
    pub(crate) fn set_i(&mut self, off: usize, v: i32) {
        self.0[off >> 2] = v as u32;
    }
    #[inline(always)]
    pub(crate) fn d(&self, off: usize) -> f64 {
        let k = off >> 2;
        f64::from_bits(self.0[k] as u64 | (self.0[k + 1] as u64) << 32)
    }
    #[inline(always)]
    pub(crate) fn set_d(&mut self, off: usize, v: f64) {
        let (k, b) = (off >> 2, v.to_bits());
        self.0[k] = b as u32;
        self.0[k + 1] = (b >> 32) as u32;
    }
    /// Zero `len` bytes from `off` (the kernels' `bzero`s).
    pub(crate) fn zero(&mut self, off: usize, len: usize) {
        self.0[off >> 2..(off + len) >> 2].fill(0);
    }
}

/// `frintx` + `fcvtzs`: round to nearest, ties to even (the FPU's mode), to an integer.
#[inline(always)]
pub(crate) fn rne(x: f32) -> i32 {
    x.round_ties_even() as i32
}

/// [`rne`] for `f64`.
#[inline(always)]
pub(crate) fn rne_d(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// `2^k` as an `f32` built in the exponent bits (`0x3f800000 + (k << 23)`).
#[inline(always)]
pub(crate) fn pow2i(k: i32) -> f32 {
    f32::from_bits(0x3f80_0000u32.wrapping_add((k as u32) << 23))
}

/// Entry `i` of the kernels' octave table (the one Chorus, Flanger and Frequency Shifter
/// use, docs/fx-chorus.md §3.1): `2^(i/1021 − 0.5)` to 6 significant digits, with seven
/// near-ties rounded the other way and the entries from 1022 on stored as √2.
pub(crate) fn exp2_entry(i: usize) -> f32 {
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

/// The octave table, entries 0..=1022.
pub(crate) struct Exp2Table(pub(crate) [f32; 1023]);

impl Exp2Table {
    pub(crate) fn new() -> Exp2Table {
        Exp2Table(std::array::from_fn(exp2_entry))
    }

    /// `2^x` as the kernels compute it: the fraction (about −0.5..0.5 octave) through
    /// the table with linear interpolation, the integer part into the exponent bits.
    /// (The kernels write the interpolation's two products in either order; the sum is
    /// the same.)
    #[inline(always)]
    pub(crate) fn exp2(&self, x: f32) -> f32 {
        let n = rne(x);
        let y = ((x - n as f32) + 0.5) * 1021.0;
        let k = rne(y + -0.5);
        let fr = y - k as f32;
        let i = (k as u32 & 0x3ff) as usize;
        let (a, b) = (self.0[i], self.0[i + 1]);
        let v = a * (1.0 - fr) + b * fr;
        v * pow2i(n)
    }
}

/// `RxEnvAHD`'s double-precision power-of-two table (`s_00000000`): `2^(i − 128)` to 6
/// significant digits, `i` = 0..=256.
#[allow(clippy::approx_constant)]
pub(crate) static POW2_D: [f64; 257] = [
    2.93874e-39,
    5.87747e-39,
    1.17549e-38,
    2.35099e-38,
    4.70198e-38,
    9.40395e-38,
    1.88079e-37,
    3.76158e-37,
    7.52316e-37,
    1.50463e-36,
    3.00927e-36,
    6.01853e-36,
    1.20371e-35,
    2.40741e-35,
    4.81482e-35,
    9.62965e-35,
    1.92593e-34,
    3.85186e-34,
    7.70372e-34,
    1.54074e-33,
    3.08149e-33,
    6.16298e-33,
    1.2326e-32,
    2.46519e-32,
    4.93038e-32,
    9.86076e-32,
    1.97215e-31,
    3.9443e-31,
    7.88861e-31,
    1.57772e-30,
    3.15544e-30,
    6.31089e-30,
    1.26218e-29,
    2.52435e-29,
    5.04871e-29,
    1.00974e-28,
    2.01948e-28,
    4.03897e-28,
    8.07794e-28,
    1.61559e-27,
    3.23117e-27,
    6.46235e-27,
    1.29247e-26,
    2.58494e-26,
    5.16988e-26,
    1.03398e-25,
    2.06795e-25,
    4.1359e-25,
    8.27181e-25,
    1.65436e-24,
    3.30872e-24,
    6.61744e-24,
    1.32349e-23,
    2.64698e-23,
    5.29396e-23,
    1.05879e-22,
    2.11758e-22,
    4.23516e-22,
    8.47033e-22,
    1.69407e-21,
    3.38813e-21,
    6.77626e-21,
    1.35525e-20,
    2.71051e-20,
    5.42101e-20,
    1.0842e-19,
    2.1684e-19,
    4.33681e-19,
    8.67362e-19,
    1.73472e-18,
    3.46945e-18,
    6.93889e-18,
    1.38778e-17,
    2.77556e-17,
    5.55112e-17,
    1.11022e-16,
    2.22045e-16,
    4.44089e-16,
    8.88178e-16,
    1.77636e-15,
    3.55271e-15,
    7.10543e-15,
    1.42109e-14,
    2.84217e-14,
    5.68434e-14,
    1.13687e-13,
    2.27374e-13,
    4.54747e-13,
    9.09495e-13,
    1.81899e-12,
    3.63798e-12,
    7.27596e-12,
    1.45519e-11,
    2.91038e-11,
    5.82077e-11,
    1.16415e-10,
    2.32831e-10,
    4.65661e-10,
    9.31323e-10,
    1.86265e-09,
    3.72529e-09,
    7.45058e-09,
    1.49012e-08,
    2.98023e-08,
    5.96046e-08,
    1.19209e-07,
    2.38419e-07,
    4.76837e-07,
    9.53674e-07,
    1.90735e-06,
    3.8147e-06,
    7.62939e-06,
    1.52588e-05,
    3.05176e-05,
    6.10352e-05,
    0.00012207,
    0.000244141,
    0.000488281,
    0.000976563,
    0.00195313,
    0.00390625,
    0.0078125,
    0.015625,
    0.03125,
    0.0625,
    0.125,
    0.25,
    0.5,
    1.0,
    2.0,
    4.0,
    8.0,
    16.0,
    32.0,
    64.0,
    128.0,
    256.0,
    512.0,
    1024.0,
    2048.0,
    4096.0,
    8192.0,
    16384.0,
    32768.0,
    65536.0,
    131072.0,
    262144.0,
    524288.0,
    1048580.0,
    2097150.0,
    4194300.0,
    8388610.0,
    16777200.0,
    33554400.0,
    67108900.0,
    134218000.0,
    268435000.0,
    536871000.0,
    1073740000.0,
    2147480000.0,
    4294970000.0,
    8589930000.0,
    17179900000.0,
    34359700000.0,
    68719500000.0,
    137439000000.0,
    274878000000.0,
    549756000000.0,
    1099510000000.0,
    2199020000000.0,
    4398050000000.0,
    8796090000000.0,
    17592200000000.0,
    35184400000000.0,
    70368700000000.0,
    140737000000000.0,
    281475000000000.0,
    562950000000000.0,
    1125900000000000.0,
    2251800000000000.0,
    4503600000000000.0,
    9007200000000000.0,
    1.80144e+16,
    3.60288e+16,
    7.20576e+16,
    1.44115e+17,
    2.8823e+17,
    5.76461e+17,
    1.15292e+18,
    2.30584e+18,
    4.61169e+18,
    9.22337e+18,
    1.84467e+19,
    3.68935e+19,
    7.3787e+19,
    1.47574e+20,
    2.95148e+20,
    5.90296e+20,
    1.18059e+21,
    2.36118e+21,
    4.72237e+21,
    9.44473e+21,
    1.88895e+22,
    3.77789e+22,
    7.55579e+22,
    1.51116e+23,
    3.02231e+23,
    6.04463e+23,
    1.20893e+24,
    2.41785e+24,
    4.8357e+24,
    9.67141e+24,
    1.93428e+25,
    3.86856e+25,
    7.73713e+25,
    1.54743e+26,
    3.09485e+26,
    6.1897e+26,
    1.23794e+27,
    2.47588e+27,
    4.95176e+27,
    9.90352e+27,
    1.9807e+28,
    3.96141e+28,
    7.92282e+28,
    1.58456e+29,
    3.16913e+29,
    6.33825e+29,
    1.26765e+30,
    2.5353e+30,
    5.0706e+30,
    1.01412e+31,
    2.02824e+31,
    4.05648e+31,
    8.11296e+31,
    1.62259e+32,
    3.24519e+32,
    6.49037e+32,
    1.29807e+33,
    2.59615e+33,
    5.1923e+33,
    1.03846e+34,
    2.07692e+34,
    4.15384e+34,
    8.30767e+34,
    1.66153e+35,
    3.32307e+35,
    6.64614e+35,
    1.32923e+36,
    2.65846e+36,
    5.31691e+36,
    1.06338e+37,
    2.12676e+37,
    4.25353e+37,
    8.50706e+37,
    1.70141e+38,
    3.40282e+38,
];

/// `RxEnvAHD`'s sixteenth-octave table (`s_00000001`): `2^((k − 8)/16)` to 6 significant
/// digits.
#[allow(clippy::approx_constant)]
pub(crate) static POW2_16: [f64; 17] = [
    0.707107, 0.738413, 0.771105, 0.805245, 0.840896, 0.878126, 0.917004, 0.957603, 1.0, 1.04427, 1.09051, 1.13879, 1.18921, 1.24186,
    1.29684, 1.35426, 1.41421,
];
