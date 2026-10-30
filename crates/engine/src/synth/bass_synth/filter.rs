//! `NI::DSP::TSKFilterNLA<float, 1>` (docs/synth-bass-synth.md): the Bass Synth's
//! low-pass filter, a nonlinear two-pole Sallen-Key ("TSK") in trapezoidal form. Every
//! feedback path runs through a scaled `tanh` (an `exp`-based approximation, linear below
//! |x| = 0.001 and hard-limited above 10). Follows `setFreq` (0x1019c3e54) and
//! `processInput` (0x1019c410c) operation by operation; unlike the Reaktor kernels this
//! one is C++ and the compiler fused some products into `fmadd`/`fmsub` (`mul_add` here,
//! one `f64` fused step included).
//!
//! State (byte offsets, as in Maschine; the block sits at BassSynthCore+0x3290):
//!
//! | off | field |
//! |---|---|
//! | 0x00 | `1/sr` (oversampled rate) |
//! | 0x04 | `g`: prewarped cutoff, `atan(min(π·f/sr, 1.50845))` |
//! | 0x08 | cutoff frequency (Hz) |
//! | 0x0c | `k`: feedback (Resonance·1.2·2, at most 4) |
//! | 0x10, 0x18 | output scales of the two `tanh` kinds (`a·tanh(x/a)` pairs) |
//! | 0x14, 0x1c | input scales of the two `tanh` kinds |
//! | 0x20 | `α`: mix of the integrator differences in the first estimate |
//! | 0x24 | `g(g+1) / ((2 − k + g)·g + 1)` |
//! | 0x28 | `g/(g+1)` |
//! | 0x2c | `1/g` |
//! | 0x30, 0x34 | integrator states `s1`, `s2` (0x34 is the output) |
//! | 0x38, 0x3c | their saturated values `T(s1)`, `T(s2)` |
//! | 0x40 | the previous input |

use super::rx::{Mem, pow2i};

/// The kernel's state, laid out as in Maschine.
pub(crate) struct Filter {
    pub(crate) m: Mem<{ 0x48 / 4 }>,
}

/// `scale·tanh(x)` for `x = arg` (`arg` already multiplied by the input scale): `arg·scale`
/// below |arg| = 0.001 (compared in `f64`), `±scale` (sign of `sign`) from |arg| = 10, and
/// between them `(e − 1)/(e + 1)·scale` with `e = 2^(2·arg/ln2)` from a cubic.
#[inline(always)]
#[allow(clippy::neg_cmp_op_on_partial_ord)] // the kernel's NaN behaviour
fn sat(arg: f32, scale: f32, sign: f32) -> f32 {
    let a = arg.abs();
    if (a as f64) < 0.001 {
        return arg * scale;
    }
    if !(a < 10.0) {
        return scale.abs().copysign(sign);
    }
    let t = arg * f32::from_bits(0x4038_aa3b); // 2.88539 = 2/ln 2
    let r = t.round_ties_even(); // frinti
    let n = if r < 2_147_483_648.0 && r > -2_147_483_648.0 { r as i32 } else { i32::MIN };
    let f = t - n as f32;
    let p = f.mul_add(f32::from_bits(0x3d65_6042), f32::from_bits(0x3e77_ced9)); // 0.056, 0.242
    let p = p.mul_add(f, f32::from_bits(0x3f31_6873)); // 0.693
    let p = p.mul_add(f, 1.0) * pow2i(n);
    (p + -1.0) / (p + 1.0) * scale
}

/// `atan(x)` as `setFreq` (and BassSynthCore) computes it: a degree-15 odd polynomial
/// on |x| ≤ 1, `±π/2 − atan(1/x)` outside.
pub(crate) fn atan(x: f32) -> f32 {
    fn poly(x: f32) -> f32 {
        let x2 = x * x;
        let mut p = x * f32::from_bits(0x3b3b_d75c) + f32::from_bits(0xbc84_6dee);
        for c in [0x3d2f_c1fau32, 0xbd9a_316f, 0x3dda_3db4, 0xbe11_7fc7, 0x3e4c_bc06, 0xbeaa_aa5c] {
            p = x2 * p + f32::from_bits(c);
        }
        x * (x2 * p + 1.0)
    }
    if x > 1.0 {
        f32::from_bits(0x3fc9_0ff9) - poly(1.0 / x) // 1.5708
    } else if x < -1.0 {
        f32::from_bits(0xbfc9_0ff9) - poly(1.0 / x)
    } else {
        poly(x)
    }
}

impl Filter {
    pub(crate) fn new() -> Filter {
        Filter { m: Mem::new() }
    }

    /// The output (`s2`).
    pub(crate) fn out(&self) -> f32 {
        self.m.f(0x34)
    }

    /// `setFreq`: cutoff `f` Hz → `g` and the derived coefficients (with `k` as stored).
    pub(crate) fn set_freq(&mut self, f: f32) {
        let m = &mut self.m;
        let k = m.f(0xc);
        m.set_f(0x8, f);
        let x = f * f32::from_bits(0x4049_0fdb) * m.f(0x0);
        let lim = f32::from_bits(0x3fc1_14e4); // 1.50845
        let x = if x > lim { lim } else { x };
        let g = atan(x);
        m.set_f(0x4, g);
        let g1 = g + 1.0;
        let den = g.mul_add((2.0 - k) + g, 1.0);
        m.set_f(0x24, g * g1 / den);
        m.set_f(0x28, g / g1);
        m.set_f(0x2c, 1.0 / g);
    }

    /// `processInput`: one sample `x` through the filter.
    pub(crate) fn process_input(&mut self, x: f32) {
        let m = &mut self.m;
        let (k, g) = (m.f(0xc), m.f(0x4));
        let (oa, ia, ob, ib) = (m.f(0x10), m.f(0x14), m.f(0x18), m.f(0x1c));
        let (alpha, c24, c28, c2c) = (m.f(0x20), m.f(0x24), m.f(0x28), m.f(0x2c));
        let (s1, s2, t1, t2, xp) = (m.f(0x30), m.f(0x34), m.f(0x38), m.f(0x3c), m.f(0x40));
        let ad = alpha as f64;
        let d12 = s1 - s2;
        let dt = t1 - t2;
        // First estimate of the feedback, from last sample's state.
        let e0 = ad.mul_add((dt - d12) as f64, d12 as f64) as f32;
        let u = c28 * c2c.mul_add(s2, dt);
        let fb = sat(ia * u, oa, u);
        let v = k.mul_add(-fb, x);
        let a1 = sat(ib * v, ob, v);
        let w = k.mul_add(e0, xp);
        let wa = ib * w;
        let a2 = sat(wa, ob, w);
        let h1 = c24 * c2c.mul_add(s1, (a1 + a2) - t1);
        let b1 = sat(ia * h1, oa, h1);
        let h2 = c28 * c2c.mul_add(s2, (t1 + b1) - t2);
        let b2 = sat(ia * h2, oa, h2);
        // Refined feedback, then the trapezoidal integrators.
        let e1 = ad.mul_add(((b1 - b2) - (h1 - h2)) as f64, (h1 - h2) as f64) as f32;
        let y = k.mul_add(e1, x);
        let a3 = sat(ib * y, ob, y);
        let a2 = sat(wa, ob, w);
        let n1 = g.mul_add(((a3 + a2) - t1) - b1, s1);
        let nt1 = sat(ia * n1, oa, n1);
        let n2 = g.mul_add(((t1 + nt1) - t2) - b2, s2);
        let nt2 = sat(ia * n2, oa, n2);
        m.set_f(0x30, n1);
        m.set_f(0x34, n2);
        m.set_f(0x38, nt1);
        m.set_f(0x3c, nt2);
        m.set_f(0x40, x);
    }
}
