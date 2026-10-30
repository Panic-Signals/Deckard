//! Building blocks shared by the Percussion engines (docs/synth-perc.md §3): the kernel
//! state image, Maschine's exponential table, NEON reciprocal estimates, the 4-lane
//! helpers the engines' vector code uses, the parameter smoother and `FreeEnvelope`.
//!
//! Maschine's drum-synth cores are C++ classes whose per-sample code is heavily inlined
//! and vectorised. The ports keep each core's state as a byte image laid out like the
//! class (offsets as in the disassembly, named by constants in each engine), so every
//! load and store of the original maps to one accessor here and the arithmetic can follow
//! the assembly operation by operation (`f32`, `f64` where it uses doubles, `mul_add`
//! exactly where it fuses).

use std::sync::OnceLock;

/// A core's state: a zeroed byte image with typed accessors at byte offsets.
#[derive(Clone)]
pub struct Mem {
    b: Vec<u8>,
}

impl Mem {
    pub fn new(len: usize) -> Mem {
        Mem { b: vec![0; len] }
    }
    #[inline]
    fn w<const N: usize>(&self, o: usize) -> [u8; N] {
        self.b[o..o + N].try_into().unwrap()
    }
    #[inline]
    pub fn f(&self, o: usize) -> f32 {
        f32::from_le_bytes(self.w(o))
    }
    #[inline]
    pub fn set_f(&mut self, o: usize, v: f32) {
        self.b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn d(&self, o: usize) -> f64 {
        f64::from_le_bytes(self.w(o))
    }
    #[inline]
    pub fn set_d(&mut self, o: usize, v: f64) {
        self.b[o..o + 8].copy_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn u(&self, o: usize) -> u32 {
        u32::from_le_bytes(self.w(o))
    }
    #[inline]
    pub fn set_u(&mut self, o: usize, v: u32) {
        self.b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn set_x(&mut self, o: usize, v: u64) {
        self.b[o..o + 8].copy_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn b8(&self, o: usize) -> u8 {
        self.b[o]
    }
    #[inline]
    pub fn set_b8(&mut self, o: usize, v: u8) {
        self.b[o] = v;
    }
    #[inline]
    pub fn q(&self, o: usize) -> V4 {
        [self.f(o), self.f(o + 4), self.f(o + 8), self.f(o + 12)]
    }
    #[inline]
    pub fn set_q(&mut self, o: usize, v: V4) {
        for (i, x) in v.iter().enumerate() {
            self.set_f(o + 4 * i, *x);
        }
    }
    /// Copy `n` bytes from `from` to `to` (a 16-byte `ldr q`/`str q` pair when n = 16).
    pub fn copy(&mut self, from: usize, to: usize, n: usize) {
        self.b.copy_within(from..from + n, to);
    }
    /// Zero `n` bytes at `o`.
    pub fn zero(&mut self, o: usize, n: usize) {
        self.b[o..o + n].fill(0);
    }
}

/// Four `f32` lanes (a NEON `.4s` register).
pub type V4 = [f32; 4];

#[inline]
pub fn splat(x: f32) -> V4 {
    [x; 4]
}
#[inline]
pub fn vadd(a: V4, b: V4) -> V4 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}
#[inline]
pub fn vsub(a: V4, b: V4) -> V4 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]]
}
#[inline]
pub fn vmul(a: V4, b: V4) -> V4 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}
#[inline]
pub fn vabs(a: V4) -> V4 {
    a.map(f32::abs)
}
/// NEON `fmin` (NaN wins; −0 < +0).
#[inline]
pub fn fmin(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a < b || (a == b && a.is_sign_negative()) {
        a
    } else {
        b
    }
}
/// NEON `fmax` (NaN wins; +0 > −0).
#[inline]
pub fn fmax(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a > b || (a == b && a.is_sign_positive()) {
        a
    } else {
        b
    }
}
#[inline]
pub fn vmin(a: V4, b: V4) -> V4 {
    [fmin(a[0], b[0]), fmin(a[1], b[1]), fmin(a[2], b[2]), fmin(a[3], b[3])]
}
#[inline]
pub fn vmax(a: V4, b: V4) -> V4 {
    [fmax(a[0], b[0]), fmax(a[1], b[1]), fmax(a[2], b[2]), fmax(a[3], b[3])]
}
/// `fcmp x, #0; fcsel lo, x, mi` then `fcmp x, hi; fcsel hi, x, gt`: clamp that keeps NaN.
#[inline]
pub fn clamp01(x: f32) -> f32 {
    let x = if x < 0.0 { 0.0 } else { x };
    if x > 1.0 { 1.0 } else { x }
}

/// NEON `frecpe` (single precision, FPCR.AH = 0): an 8-bit reciprocal estimate.
pub fn frecpe(x: f32) -> f32 {
    let bits = x.to_bits();
    let sign = bits & 0x8000_0000;
    let exp = ((bits >> 23) & 0xff) as i32;
    let frac = bits & 0x7f_ffff;
    if x.is_nan() {
        return f32::from_bits(bits | 0x40_0000);
    }
    if exp == 0xff {
        return f32::from_bits(sign);
    }
    if exp == 0 && frac == 0 {
        return f32::from_bits(sign | 0x7f80_0000);
    }
    // |x| < 2^-128: the estimate overflows (round to nearest: infinity).
    if x.abs() < f32::from_bits(0x0020_0000) {
        return f32::from_bits(sign | 0x7f80_0000);
    }
    let (mut e, mut f) = (exp, frac);
    if e == 0 {
        if f & (1 << 22) == 0 {
            e = -1;
            f = (f & 0x1f_ffff) << 2;
        } else {
            f = (f & 0x3f_ffff) << 1;
        }
    }
    let scaled = 256 + (f >> 15); // '1':fraction<22:15>
    let a = scaled * 2 + 1;
    let b = (1u32 << 19) / a;
    let r = (b + 1) / 2; // 256..511
    let mut res_exp = 253 - e;
    let mut res_frac = (r & 0xff) << 15;
    if res_exp == 0 {
        res_frac = (1 << 22) | (res_frac >> 1);
    } else if res_exp == -1 {
        res_frac = (1 << 21) | (res_frac >> 2);
        res_exp = 0;
    }
    f32::from_bits(sign | ((res_exp as u32 & 0xff) << 23) | res_frac)
}

/// NEON `frecps`: `2 − a·b` with one rounding.
#[inline]
pub fn frecps(a: f32, b: f32) -> f32 {
    if (a.is_infinite() && b == 0.0) || (a == 0.0 && b.is_infinite()) {
        return 2.0;
    }
    (-a).mul_add(b, 2.0)
}

/// `1/x` as the cores compute it: `frecpe` and two Newton steps, `e·s` then `s·e`.
#[inline]
pub fn recip(x: f32) -> f32 {
    let e = frecpe(x);
    let e = e * frecps(x, e);
    frecps(x, e) * e
}

#[inline]
pub fn vrecip(v: V4) -> V4 {
    v.map(recip)
}

/// `NI::AB::ExpTable` (§3.1): 2401 pairs (value, slope), `value[k] = 2^(k/60 − 20)`.
pub struct ExpTable {
    v: Vec<f32>,
}

impl ExpTable {
    fn build() -> ExpTable {
        // Built once at startup by a static initialiser: value[0] = 2^-20, then for
        // k = 1..=2400 `(f32)exp2(k/60.0 − 20.0)` (f64 libm), slope[k−1] = value[k] −
        // value[k−1] (f32), slope[2400] = 0.
        let mut v = vec![0f32; 2 * 2401];
        v[0] = f32::from_bits(0x3580_0000);
        for k in 1..=2400usize {
            let x = ((k as f64) / 60.0 + -20.0).exp2() as f32;
            v[2 * k] = x;
            v[2 * k - 1] = x - v[2 * k - 2];
        }
        v[2 * 2400 + 1] = 0.0;
        ExpTable { v }
    }
    /// The table's interpolated lookup at index `x` (`fcvtzs`; index ≥ 2400 gives the
    /// last value, negative gives 0).
    #[inline]
    pub fn at(&self, x: f32) -> f32 {
        let i = x as i32;
        if i >= 0x960 {
            self.v[2 * 2400]
        } else if i < 0 {
            0.0
        } else {
            let k = (i as usize) & 0xffff;
            (x - i as f32).mul_add(self.v[2 * k + 1], self.v[2 * k])
        }
    }
    /// `x` dB as a gain: `at(x·9.965784 + 1200)`.
    #[inline]
    pub fn db(&self, x: f32) -> f32 {
        self.at(x.mul_add(LOG_STEP, 1200.0))
    }
    /// `x` dB below 60 (a time in ms as seconds, ×10^-3): `at(x·9.965784 + 602.0529)`.
    #[inline]
    pub fn db60(&self, x: f32) -> f32 {
        self.at(x.mul_add(LOG_STEP, f32::from_bits(0x4416_8363)))
    }
    /// MIDI note `x` in Hz: `at(x·5 + 1381.8816)`.
    #[inline]
    pub fn hz(&self, x: f32) -> f32 {
        self.at(x.mul_add(5.0, f32::from_bits(0x44ac_bc36)))
    }
}

/// 60 table steps per octave per 20·log10(2) dB (0x411f73da = 9.965784).
pub const LOG_STEP: f32 = f32::from_bits(0x411f_73da);

static EXP: OnceLock<ExpTable> = OnceLock::new();

/// The shared table (built on first use; [`crate::synth::perc::Perc::new`] builds it off
/// the audio thread).
pub fn exp_table() -> &'static ExpTable {
    EXP.get_or_init(ExpTable::build)
}

/// `−2.07944` (ln 8, 0xc000a2b1704ff434): log decays reach 1/8 after their time.
pub const LN8_NEG: f64 = f64::from_bits(0xc000_a2b1_704f_f434);
/// `1e30` (0x46293e5939a08cea): "no time limit".
pub const NEVER: f64 = f64::from_bits(0x4629_3e59_39a0_8cea);
/// `1e-4` as the envelopes compare it (0x3f1a36e2eb1c432d).
pub const ENV_EPS: f64 = f64::from_bits(0x3f1a_36e2_eb1c_432d);

/// `tan(w)` as the cores approximate it: `w + w³/3 + 2w⁵/15` (two constant spellings
/// occur: 0x3eaaaaab in the vector code, 0x3eaaaaaa in the scalar code).
#[inline]
pub fn tan_scalar(w: f32) -> f32 {
    let w2 = w * w;
    let w3 = w * w2;
    let w5 = w2 * w3;
    let t = w3 * f32::from_bits(0x3eaa_aaaa);
    w + w5.mul_add(f32::from_bits(0x3e08_8886), t)
}

#[inline]
pub fn tan_v(w: V4) -> V4 {
    w.map(|w| {
        let w2 = w * w;
        let w3 = w * w2;
        let w5 = w2 * w3;
        let a = w5 * f32::from_bits(0x3e08_8886);
        let b = w3 * f32::from_bits(0x3eaa_aaab);
        w + (b + a)
    })
}

/// The vector `2^x` (NEON code, no fused operations): round `x` to the nearest integer
/// `n` (halves away from zero, |x| ≥ 2^24 kept), `f = x − n`, and
/// `(1 + f·(0.693 + f·(0.242 + f·0.056)))·2^n`.
#[inline]
pub fn exp2_v(x: f32) -> f32 {
    let n = round_lane(x);
    let f = x - n as f32;
    let p2 = f32::from_bits(((n as u32) << 23).wrapping_add(0x3f80_0000));
    let a = f * f32::from_bits(0x3d65_6042) + f32::from_bits(0x3e77_ced9);
    let a = f * a + f32::from_bits(0x3f31_6873);
    (f * a + 1.0) * p2
}

/// The NEON round-to-nearest used by [`exp2_v`] (fcmgt/bsl sequence): returns `n`.
#[inline]
pub fn round_lane(x: f32) -> i32 {
    let big = f32::from_bits(0x4b80_0000); // 2^24
    let ax = x.abs();
    let t = if big > ax { (ax as i32) as f32 } else { ax };
    let c = if ax > t { t + 1.0 } else { t };
    let r = if c + -0.5 > ax { c + -1.0 } else { c };
    let s = f32::from_bits((x.to_bits() & 0x8000_0000) | r.abs().to_bits());
    let v = if ax > big { x } else { s };
    v as i32
}

/// The vector `log2(x)` (NEON code): exponent plus `m·(1.34625 − m·0.346247)` of the
/// mantissa `m − 1` (sign of `x` kept in `m`).
#[inline]
pub fn log2_v(x: f32) -> f32 {
    let m = f32::from_bits((x.to_bits() & 0x807f_ffff) | 0x3f80_0000) + -1.0;
    let p = m * (f32::from_bits(0x3fac_51ec) - m * f32::from_bits(0x3eb1_4749));
    let e = (((x.to_bits() >> 23) & !0x100) as i32).wrapping_add(-127) as f32;
    p + e
}

/// `NI::{DSP,AB}::SmoothingToolbox<N>` as the cores embed it: `n` values (lanes, padded
/// to a multiple of 4) ramped linearly to their targets over `steps` control periods. With
/// `n` lanes from `b`: increment +4n, target +8n, start flag +12n, active flag +12n+1,
/// counter +12n+4, steps +12n+8, 1/(steps·period) +12n+12, state +12n+16.
pub mod smooth {
    use super::Mem;

    pub fn control(m: &mut Mem, b: usize, n: usize) {
        let (inc, tgt, fl) = (b + 4 * n, b + 8 * n, b + 12 * n);
        if m.b8(fl) != 0 {
            m.set_b8(fl, 0);
            m.set_u(fl + 16, 2);
            let k = m.f(fl + 12);
            let mut any = false;
            for i in 0..n {
                let d = (m.f(tgt + 4 * i) - m.f(b + 4 * i)) * k;
                m.set_f(inc + 4 * i, d);
                if ((d * d) as f64) < 1e-15 {
                    m.set_f(inc + 4 * i, 0.0);
                    m.set_f(b + 4 * i, m.f(tgt + 4 * i));
                } else {
                    any = true;
                }
            }
            if any {
                m.set_u(fl + 4, m.u(fl + 8));
                m.set_b8(fl + 1, 1);
            } else {
                m.set_u(fl + 16, 4);
            }
            return;
        }
        let c = m.u(fl + 4).wrapping_sub(1);
        m.set_u(fl + 4, c);
        if c == 0 {
            m.copy(tgt, b, 4 * n);
            m.zero(inc, 4 * n);
            m.set_b8(fl + 1, 0);
            m.set_u(fl + 16, 4);
            return;
        }
        match m.u(fl + 16) {
            2 => m.set_u(fl + 16, 3),
            4 => m.set_u(fl + 16, 0),
            _ => {}
        }
    }

    /// One sample of an active ramp: `current += increment`.
    #[inline]
    pub fn step(m: &mut Mem, b: usize, n: usize) {
        for i in 0..n {
            let v = m.f(b + 4 * i) + m.f(b + 4 * n + 4 * i);
            m.set_f(b + 4 * i, v);
        }
    }

    #[inline]
    pub fn active(m: &Mem, b: usize, n: usize) -> bool {
        m.b8(b + 12 * n + 1) != 0
    }

    /// Start a ramp to the targets: flags only.
    #[inline]
    pub fn start(m: &mut Mem, b: usize, n: usize) {
        m.set_b8(b + 12 * n, 1);
        m.set_u(b + 12 * n + 16, 1);
    }

    /// Jump to the targets.
    #[inline]
    pub fn snap(m: &mut Mem, b: usize, n: usize) {
        m.copy(b + 8 * n, b, 4 * n);
        start(m, b, n);
    }

    /// `steps = max(round(sr·0.02/period), 1)`.
    pub fn set_rate(m: &mut Mem, b: usize, n: usize, sr: f32, period: i32) {
        let s = ((sr * f32::from_bits(0x3ca3_d70a)) / period as f32 + 0.5) as i32;
        let s = if s > 1 { s } else { 1 };
        m.set_u(b + 12 * n + 8, s as u32);
        m.set_f(b + 12 * n + 12, 1.0 / (s * period) as f32);
    }
}

/// The 8-lane smoother (Fractal).
pub mod smooth8 {
    use super::{Mem, smooth};
    pub fn control(m: &mut Mem, b: usize) {
        smooth::control(m, b, 8)
    }
    pub fn step(m: &mut Mem, b: usize) {
        smooth::step(m, b, 8)
    }
    pub fn start(m: &mut Mem, b: usize) {
        smooth::start(m, b, 8)
    }
    pub fn snap(m: &mut Mem, b: usize) {
        smooth::snap(m, b, 8)
    }
    pub fn set_rate(m: &mut Mem, b: usize, sr: f32, period: i32) {
        smooth::set_rate(m, b, 8, sr, period)
    }
}

/// The TPT state-variable filter the cores embed at `b`: +0 band state, +4 low state, +8
/// high-pass out, +0xc band sum, +0x10 low sum, +0x14 g, +0x18 k, +0x1c frequency, +0x24
/// 1/(1 + g(g + k)), +0x28 sample rate, +0x2c 1/sr. Returns the high-pass output.
pub mod svf {
    use super::{Mem, tan_scalar};

    #[inline]
    pub fn tick(m: &mut Mem, b: usize, x: f32) -> f32 {
        let (g, k, d) = (m.f(b + 0x14), m.f(b + 0x18), m.f(b + 0x24));
        tick_with(m, b, x, g, k, d)
    }

    #[inline]
    pub fn tick_with(m: &mut Mem, b: usize, x: f32, g: f32, k: f32, d: f32) -> f32 {
        let b0 = m.f(b);
        let l0 = m.f(b + 4);
        let b2 = b0 + b0;
        let a = l0.mul_add(2.0, g * b2);
        let gg = g.mul_add(k, g * g);
        let a = b2.mul_add(k, a);
        let a = x.mul_add(gg, a);
        let hp = x - d * a;
        m.set_f(b + 8, hp);
        let nb = g.mul_add(hp, b0);
        let bs = b0 + nb;
        m.set_f(b + 0xc, bs);
        let nl = g.mul_add(bs, l0);
        m.set_f(b + 0x10, l0 + nl);
        m.set_f(b, nb);
        m.set_f(b + 4, nl);
        hp
    }

    /// Set the cutoff (only when it changed): clamp to 0..0.49·sr, `g = tan(π f/sr)`.
    #[inline]
    pub fn set_freq(m: &mut Mem, b: usize, f: f32) {
        if m.f(b + 0x1c) == f {
            return;
        }
        let lim = m.f(b + 0x28) * f32::from_bits(0x3efa_e148);
        let f = if f < 0.0 { 0.0 } else { f };
        let f = if f > lim { lim } else { f };
        m.set_f(b + 0x1c, f);
        let g = tan_scalar(m.f(b + 0x2c) * (f * std::f32::consts::PI));
        m.set_f(b + 0x14, g);
        let k = m.f(b + 0x18);
        m.set_f(b + 0x24, 1.0 / g.mul_add(k + g, 1.0));
    }

    /// `setSampleRate`: store sr and 1/sr, re-apply the cutoff unless unset (−1), clear.
    pub fn set_rate(m: &mut Mem, b: usize, sr: f32, inv: f32) {
        m.set_f(b + 0x28, sr);
        m.set_f(b + 0x2c, inv);
        let f = m.f(b + 0x1c);
        m.set_f(b + 0x1c, -1.0);
        if f != -1.0 {
            let lim = sr * f32::from_bits(0x3efa_e148);
            let f = if f < 0.0 { 0.0 } else { f };
            let f = if f > lim { lim } else { f };
            m.set_f(b + 0x1c, f);
            let g = tan_scalar(inv * (f * std::f32::consts::PI));
            m.set_f(b + 0x14, g);
            let k = m.f(b + 0x18);
            m.set_f(b + 0x24, 1.0 / g.mul_add(g + k, 1.0));
        }
        m.zero(b, 0x14);
    }

    /// The constructor's "k changed" update: k = 2 (if the field at +0x20 was set).
    pub fn init_k(m: &mut Mem, b: usize) {
        if m.f(b + 0x20) != 0.0 {
            m.set_f(b + 0x20, 0.0);
            m.set_f(b + 0x18, 2.0);
            let g = m.f(b + 0x14);
            m.set_f(b + 0x24, 1.0 / g.mul_add(g + 2.0, 1.0));
        }
    }
}

/// The per-sample factor of the cores' exponential decays: `clamp01(2^((−ln2/sr)/(t +
/// 1e-30)·86.5617/60 − 0))` via the table (`t` in seconds).
#[inline]
pub fn decay_coef(inv: f32, t: f32) -> f32 {
    let s = inv * f32::from_bits(0xbf31_7215);
    let x = (s / (t + f32::from_bits(0x0da2_4260))).mul_add(f32::from_bits(0x42ad_1f97), 1200.0);
    clamp01(exp_table().at(x))
}

/// The scalar `2^x` of the Shaker (`frinti` rounding, fused polynomial).
#[inline]
pub fn exp2_s(x: f32) -> f32 {
    let r = x.round_ties_even();
    let n = if r < 2147483648.0 && r > -2147483648.0 { r as i32 } else { i32::MIN };
    let f = x - n as f32;
    let p = f.mul_add(f32::from_bits(0x3d65_6042), f32::from_bits(0x3e77_ced9));
    let p = p.mul_add(f, f32::from_bits(0x3f31_6873));
    let p = p.mul_add(f, 1.0);
    p * f32::from_bits(0x3f80_0000u32.wrapping_add((n as u32) << 23))
}

/// `NI::DSP::UTIL::FreeEnvelope<1,1,1,0,2>` (lin attack with a curve, lin decay to a
/// sustain breakpoint, log release), embedded at offset `b` of a core. Its stage
/// function pointer at +0 is kept as the stage number (0, 1, 2, 4).
///
/// Layout (§3.2): +0x10 stage, +0x14 level, +0x18 phase (f64, 1 → 0), +0x20 elapsed s,
/// +0x28 stage time limit, +0x30 phase step, +0x38 phase factor, +0x40 decay factor,
/// +0x48 release factor, +0x50 level at release, +0x58/+0x60 attack s and 1/s,
/// +0x68/+0x70 decay, +0x78/+0x80 release, +0x90 attack curve, +0xa8 sustain, +0xb4 gain,
/// +0xb8 velocity amount, +0xbc velocity gain, +0xe0/+0xe4/+0xe8 attack/decay/release as
/// last set (f32), +0xec sample rate, +0xf0 1/sr (f64).
pub mod env {
    use super::{ENV_EPS, LN8_NEG, Mem, NEVER};

    /// The template variants the Percussion cores use.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Kind {
        /// `<1,1,1,0,2>` (Fractal): lin attack, lin decay, log release.
        LinLog,
        /// `<1,1,1,0,1>` (Shaker Machine): lin attack, lin decay, shaped release.
        LinShape,
        /// `<1,1,2,2,0>` (Shaker Realistic): inverted-shape attack, log decay, lin release.
        InvLog,
    }
    use Kind::*;

    const C999: f64 = f64::from_bits(0x3fef_f7ce_d916_872b);

    /// The attack/release curve: `y` → `j1 + j1/(k − (y + j1)·k)` for curve `c`
    /// (`a = |c|` capped at 0.999, negated for the inverted attack), then an S-curve
    /// when `c > 0`.
    fn curve(y: f64, c: f64, invert: bool) -> f64 {
        let mut y = y;
        if c != 0.0 {
            let a = c.abs();
            let a = if invert {
                let a = if a > C999 { -C999 } else { -a };
                if a > C999 { C999 } else { a }
            } else if a > C999 {
                C999
            } else {
                a
            };
            let p = (a + 1.0) * (a + 1.0) + -1.0;
            let rp = 1.0 / p;
            let n = 1.0 - a;
            let n2 = n * n;
            let rn = 1.0 / n;
            let rn2 = rn * rn;
            let (k, j) = if a <= 0.0 { (p, rp) } else { (rn2, n2) };
            let j1 = j + 1.0;
            let z = y + j1;
            let z = (-z).mul_add(k, k);
            y = j1 + j1 / z;
        }
        if c > 0.0 {
            let h = y + -0.5;
            let h = (-h).mul_add(h.abs(), h);
            y = h.mul_add(2.0, 0.5);
        }
        y
    }

    pub fn advance(m: &mut Mem, b: usize, kind: Kind) {
        m.set_d(b + 0x18, 1.0);
        m.set_d(b + 0x20, 0.0);
        match m.u(b + 0x10) {
            0 => {
                let step = m.d(b + 0xf0) * m.d(b + 0x60);
                m.set_d(b + 0x38, 1.0);
                m.set_u(b + 0x10, 1);
                m.set_u(b, 1);
                m.set_d(b + 0x28, m.d(b + 0x58));
                m.set_d(b + 0x30, step);
            }
            1 => {
                if kind == InvLog {
                    m.set_d(b + 0x30, 0.0);
                    m.set_d(b + 0x38, m.d(b + 0x40));
                } else {
                    m.set_d(b + 0x30, m.d(b + 0xf0) * m.d(b + 0x70));
                    m.set_d(b + 0x38, 1.0);
                }
                m.set_u(b + 0x10, 2);
                m.set_u(b, 2);
                let lim = if m.f(b + 0xa8) == 0.0 { NEVER } else { m.d(b + 0x68) };
                m.set_d(b + 0x28, lim);
            }
            2 | 3 => {
                m.set_f(b + 0x50, m.f(b + 0x14));
                if kind == LinLog || m.d(b + 0xa0) == -1.0 {
                    m.set_d(b + 0x30, 0.0);
                    m.set_d(b + 0x38, m.d(b + 0x48));
                } else {
                    m.set_d(b + 0x30, m.d(b + 0xf0) * m.d(b + 0x88));
                    m.set_d(b + 0x38, 1.0);
                }
                m.set_d(b + 0x28, NEVER);
                m.set_u(b + 0x10, 4);
                m.set_u(b, 4);
            }
            4 => {
                m.set_d(b + 0x30, 0.0);
                m.set_d(b + 0x38, 1.0);
                m.set_u(b, 0);
                m.set_u(b + 0x10, 0);
                m.set_f(b + 0x14, 0.0);
                m.set_d(b + 0x18, 1.0);
            }
            _ => {}
        }
    }

    /// One sample: the level times the velocity gain and the gain.
    pub fn process(m: &mut Mem, b: usize, kind: Kind) -> f32 {
        let stage = m.u(b);
        if stage == 0 {
            return m.f(b + 0x14) * (m.f(b + 0xbc) * m.f(b + 0xb4));
        }
        let ph = (m.d(b + 0x18) - m.d(b + 0x30)) * m.d(b + 0x38);
        let t = m.d(b + 0xf0) + m.d(b + 0x20);
        let ph = if ph < 0.0 { 0.0 } else { ph };
        let ph = if ph > 1.0 { 1.0 } else { ph };
        m.set_d(b + 0x18, ph);
        m.set_d(b + 0x20, t);
        let lvl = match stage {
            1 => curve(1.0 - ph, m.d(b + 0x90), kind == InvLog) as f32,
            2 => {
                let e = ph + -ENV_EPS;
                let r = e * f64::from_bits(0x3ff0_0068_de3a_efe5) + -1.0 + 1.0;
                let r = if e < 0.0 { 0.0 } else { r };
                let s = m.f(b + 0xa8) as f64;
                (1.0 - s).mul_add(r, s) as f32
            }
            _ => {
                let e = ph + -ENV_EPS;
                let r = e * f64::from_bits(0x3ff0_0068_de3a_efe5);
                let r = if e < 0.0 { 0.0 } else { r };
                let c = m.d(b + 0xa0);
                let r = match kind {
                    LinLog => r,
                    LinShape => {
                        if c == -1.0 {
                            r
                        } else {
                            1.0 - curve(1.0 - r, c, false)
                        }
                    }
                    InvLog => {
                        if c == -1.0 {
                            r
                        } else {
                            (r + -1.0) + 1.0
                        }
                    }
                };
                (r * m.f(b + 0x50) as f64) as f32
            }
        };
        m.set_f(b + 0x14, lvl);
        if ph <= ENV_EPS || t > m.d(b + 0x28) {
            m.set_d(b + 0x18, 1.0);
            advance(m, b, kind);
        }
        m.f(b + 0x14) * (m.f(b + 0xbc) * m.f(b + 0xb4))
    }

    /// `setNoteOn(velocity)` (velocity 0..1.0001).
    pub fn note_on(m: &mut Mem, b: usize, vel: f32, kind: Kind) {
        if !(vel >= 0.0 && vel < f32::from_bits(0x3f80_0347)) {
            return;
        }
        m.set_u(b, 0);
        m.set_x(b + 0x8, 0);
        m.set_x(b + 0x10, 0);
        m.set_d(b + 0x18, 1.0);
        m.set_d(b + 0x20, 0.0);
        advance(m, b, kind);
        let a = m.f(b + 0xb8);
        m.set_f(b + 0xbc, a.mul_add(vel * vel, 1.0 - a));
    }

    /// The decay factor of a log decay (`<…,2,…>`), from the curve at +0x98.
    fn log_decay_factor(m: &Mem, b: usize, dec: f64, sr: f64) -> f64 {
        let c = m.d(b + 0x98);
        let up = c + 1.0;
        let dn = 1.0 - c;
        let r = 1.0 / dn;
        let r = if dn < f64::from_bits(0x3e7a_d7f2_9abc_af48) { f64::from_bits(0x4163_12d0_0000_0000) } else { r };
        let k = if c <= 0.0 { up } else { r };
        (LN8_NEG / ((k * dec) * sr)).exp()
    }

    /// The release time/factor update (`setReleaseTimeS` as `setSamplerate` does it).
    fn release_from(m: &mut Mem, b: usize, d8: f64, kind: Kind) {
        let rel = m.d(b + 0x78) as f32;
        m.set_f(b + 0xe8, rel);
        let d = rel as f64;
        m.set_d(b + 0x80, d);
        let c = m.d(b + 0xa0);
        if kind == LinLog || c == -1.0 {
            m.set_d(b + 0x48, (LN8_NEG / (d8 * d)).exp());
        } else {
            m.set_d(b + 0x48, 0.0);
            let q = (-c).mul_add(c, 1.0);
            let t = (d + d) / (q * q) + f64::from_bits(0x3bc7_9ca1_0000_0000);
            m.set_d(b + 0x78, t);
            m.set_d(b + 0x88, 1.0 / t);
        }
    }

    /// `setSamplerate(sr)`.
    pub fn set_rate(m: &mut Mem, b: usize, sr: f32, kind: Kind) {
        if sr <= 0.0 || sr.is_nan() {
            return;
        }
        m.set_f(b + 0xec, sr);
        let d8 = sr as f64;
        let inv = 1.0 / d8;
        m.set_d(b + 0xf0, inv);
        let s5 = 1.0 / sr;
        m.set_f(b + 0xdc, s5);
        let s4 = m.f(b + 0xd8);
        m.set_f(b + 0xd8, -1.0);
        if s4 != -1.0 {
            let w = s5 * (s4 * std::f32::consts::PI);
            let w2 = w * w;
            let w3 = w * w2;
            let w5 = w2 * w3;
            let t = w + w5.mul_add(f32::from_bits(0x3e08_8886), w3 * f32::from_bits(0x3eaa_aaaa));
            m.set_f(b + 0xd4, t);
            m.set_f(b + 0xd8, s4);
            m.set_f(b + 0xcc, 1.0 / (t + 1.0));
            m.set_f(b + 0xd0, t + -1.0);
        }
        m.set_u(b, 0);
        m.set_x(b + 0x8, 0);
        m.set_x(b + 0x10, 0);
        m.set_d(b + 0x18, 1.0);
        m.set_d(b + 0x20, 0.0);
        m.set_x(b + 0xc0, 0);
        m.set_u(b + 0xc8, 0);
        let s0 = m.d(b + 0x58) as f32;
        let s2 = inv as f32;
        let s3 = if s2 > s0 { s2 } else { s0 };
        let s3 = if s3 > 100.0 { 100.0 } else { s3 };
        m.set_d(b + 0x58, s3 as f64);
        m.set_d(b + 0x60, 1.0 / s3 as f64);
        let dec = m.d(b + 0x68) as f32;
        m.set_f(b + 0xe0, s0);
        m.set_f(b + 0xe4, dec);
        let s0 = if s2 > dec { s2 } else { dec };
        let s0 = if s0 > 100.0 { 100.0 } else { s0 };
        m.set_d(b + 0x68, s0 as f64);
        m.set_d(b + 0x70, 1.0 / s0 as f64);
        let f = if kind == InvLog { log_decay_factor(m, b, s0 as f64, d8) } else { (LN8_NEG / (d8 * s0 as f64)).exp() };
        m.set_d(b + 0x40, f);
        release_from(m, b, d8, kind);
    }

    /// The attack-time setter the cores inline (only when changed).
    pub fn set_attack(m: &mut Mem, b: usize, time: f32) {
        if m.f(b + 0xe0) == time {
            return;
        }
        m.set_f(b + 0xe0, time);
        let lo = m.d(b + 0xf0) as f32;
        let s = if time < lo { lo } else { time };
        let s = if s > 100.0 { 100.0 } else { s };
        m.set_d(b + 0x58, s as f64);
        m.set_d(b + 0x60, 1.0 / s as f64);
    }

    /// The decay-time setter the cores inline (`time` in seconds, only when changed).
    pub fn set_decay(m: &mut Mem, b: usize, time: f32, kind: Kind) {
        if m.f(b + 0xe4) == time {
            return;
        }
        m.set_f(b + 0xe4, time);
        let inv = m.d(b + 0xf0);
        let lo = inv as f32;
        let s = if time < lo { lo } else { time };
        let s = if s > 100.0 { 100.0 } else { s };
        let d = s as f64;
        let r = 1.0 / d;
        m.set_d(b + 0x68, d);
        m.set_d(b + 0x70, r);
        if kind == InvLog {
            let f = log_decay_factor(m, b, d, m.f(b + 0xec) as f64);
            m.set_d(b + 0x40, f);
            if m.u(b + 0x10) == 2 {
                m.set_d(b + 0x30, 0.0);
                m.set_d(b + 0x38, f);
                m.set_d(b + 0x28, if m.f(b + 0xa8) == 0.0 { NEVER } else { d });
            }
            return;
        }
        m.set_d(b + 0x40, (LN8_NEG / (m.f(b + 0xec) as f64 * d)).exp());
        if m.u(b + 0x10) == 2 {
            m.set_d(b + 0x30, inv * r);
            m.set_d(b + 0x38, 1.0);
            m.set_d(b + 0x28, if m.f(b + 0xa8) == 0.0 { NEVER } else { d });
        }
    }

    /// The release-time setter the cores inline.
    pub fn set_release(m: &mut Mem, b: usize, time: f32, kind: Kind) {
        if m.f(b + 0xe8) == time {
            return;
        }
        m.set_f(b + 0xe8, time);
        let d = time as f64;
        m.set_d(b + 0x80, d);
        let c = m.d(b + 0xa0);
        if kind == LinLog || c == -1.0 {
            let e = (LN8_NEG / (d * m.f(b + 0xec) as f64)).exp();
            m.set_d(b + 0x48, e);
            if m.u(b + 0x10) == 4 {
                m.set_d(b + 0x38, e);
            }
        } else {
            m.set_d(b + 0x48, 0.0);
            let q = (-c).mul_add(c, 1.0);
            let t = (d + d) / (q * q) + f64::from_bits(0x3bc7_9ca1_0000_0000);
            m.set_d(b + 0x78, t);
            let r = 1.0 / t;
            m.set_d(b + 0x88, r);
            if m.u(b + 0x10) == 4 {
                m.set_d(b + 0x30, r * m.d(b + 0xf0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp_table_values() {
        let t = exp_table();
        assert!((t.db(0.0) - 1.0).abs() < 1e-3);
        assert!((t.db(20.0) - 10.0).abs() < 1e-2);
        assert!((t.hz(69.0) - 440.0).abs() < 0.5);
        assert!((t.db60(60.0) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn frecpe_known() {
        // ARM ARM examples: frecpe(1.0) = 0.998046875, frecpe(12.0) = 0.0832519531.
        assert_eq!(frecpe(1.0), 0.998046875);
        assert_eq!(frecpe(12.0).to_bits(), 0x3daa_8000);
        assert!((recip(3.0) - 1.0 / 3.0).abs() < 1e-7);
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn frecpe_matches_hardware() {
        use std::arch::aarch64::*;
        let mut x = 1e-30f32;
        while x < 1e30 {
            for s in [x, -x, x * 1.37, x * 1.999] {
                let hw = unsafe { vgetq_lane_f32::<0>(vrecpeq_f32(vdupq_n_f32(s))) };
                assert_eq!(frecpe(s).to_bits(), hw.to_bits(), "{s}");
                let e = frecpe(s);
                let hw2 = unsafe { vgetq_lane_f32::<0>(vrecpsq_f32(vdupq_n_f32(s), vdupq_n_f32(e))) };
                assert_eq!(frecps(s, e).to_bits(), hw2.to_bits(), "{s}");
            }
            x *= 1.0123;
        }
    }
}
