//! Tom's Fractal engine: `NI::DSP::TomFBMatrixCore` (docs/synth-tom.md, Fractal).
//!
//! Four parabolic-sine oscillators frequency-modulate each other through a feedback
//! matrix (each lane's pitch is pushed by the sum of the other three lanes' filter
//! states, scaled by FM, and its level by AM), each runs through a 2-pole
//! state-variable filter, and lane 0 (soft-clipped, crossfaded by Color) is the
//! "fractal" voice. Mix crossfades it with a classic tom: a pitch-swept sine with a
//! click envelope. A noise burst through a pitch-tracking band-pass adds Impact; a 10 Hz
//! one-pole high-pass and a fixed gain finish the voice. Overlapping notes glide
//! (`MASCOMMON::SAMPLER::Glide` keeps the held notes; a held note is not retriggered).
//!
//! The core is a flat block of `f32`/`f64` state. The port keeps it as a byte image at
//! Maschine's offsets (`Core::m`), so each formula names the same location as the spec,
//! and follows the arm64 code operation by operation: `mul_add` for every fused
//! multiply-add, the NEON reciprocal estimate (`FRECPE`/`FRECPS`) in software, the
//! vector `2^x`/`log2` approximations step by step. That makes it bit-exact.

use super::Engine;
use super::dsp::exp_e;

/// Size of `TomFBMatrixCore` (the Glide it points to at +0x620 is kept separately).
const SIZE: usize = 0x640;

/// A constant by its bits, where the decimal would not round-trip.
const fn c(b: u32) -> f32 {
    f32::from_bits(b)
}
/// −ln 2 (`0xbf317215`), in the decay coefficients `E((−ln2/sr)/t · 86.5617 + 1200)`.
const LN2N: f32 = c(0xbf31_7215);
/// 1e-30, keeps `t = 0` finite.
const EPS30: f32 = c(0x0da2_4260);
/// 86.5617 = 60/ln 2: natural-log units to ExpTable steps.
const K865: f32 = c(0x42ad_1f97);
/// 1381.8816: `E(st·5 + this)` is the frequency of MIDI note `st` (Hz).
const PITCH0: f32 = c(0x44ac_bc36);
/// `E(x·9.965784 + 602.0529)`: a level in dB as a linear factor·1000.
const DB_K: f32 = c(0x411f_73da);
const DB_0: f32 = c(0x4416_8363);
const PI: f32 = c(0x4049_0fdb);
/// The tangent series constants: 1/3 (scalar 0x3eaaaaaa, vector 0x3eaaaaab), 2/15.
const THIRD: f32 = c(0x3eaa_aaaa);
const THIRD_V: f32 = c(0x3eaa_aaab);
const TWO15: f32 = c(0x3e08_8886);
/// log2(8.17742) = 3.0313597: semitones/12 + this = log2 of the frequency (MIDI 0).
const LOG2_C: f32 = c(0x4042_01cc);
/// 0.49·sr: the filters' top frequency.
const NYQ: f32 = c(0x3efa_e148);
/// 1e-4 (f64), the envelopes' end threshold.
const MIN4: f64 = f64::from_bits(0x3f1a_36e2_eb1c_432d);
/// 1e30 (f64), "never".
const BIG: f64 = f64::from_bits(0x4629_3e59_39a0_8cea);
/// −2.07944 = −3·ln 2 (f64): a −18 dB-per-time-constant exponential.
const LN8N: f64 = f64::from_bits(0xc000_a2b1_704f_f434);
const E20: f64 = f64::from_bits(0x3bc7_9ca1_0000_0000); // 1e-20

/// `clamp(x, 0, 1)` as compiled (`fcsel mi`, then `fcsel gt`; NaN passes).
#[inline]
fn c01(x: f32) -> f32 {
    clampf(x, 0.0, 1.0)
}

/// `clamp(x, lo, hi)` with the same selects.
#[inline]
fn clampf(x: f32, lo: f32, hi: f32) -> f32 {
    let a = if x < lo { lo } else { x };
    if a > hi { hi } else { a }
}

/// The same in `f64`.
#[inline]
fn clamp01d(x: f64) -> f64 {
    let a = if x < 0.0 { 0.0 } else { x };
    if a > 1.0 { 1.0 } else { a }
}

/// `tan(w)` by its series `w + w³/3 + 2w⁵/15`, as the scalar code computes it.
#[inline]
fn tan_s(w: f32) -> f32 {
    let a = w * w;
    let b = w * a;
    let a = a * b;
    let b = b * THIRD;
    w + a.mul_add(TWO15, b)
}

/// The same series as the NEON lanes compute it (no fused ops, 0.33333334).
#[inline]
fn tan_v(w: f32) -> f32 {
    let a = w * w;
    let b = w * a;
    let a = a * b;
    let a = a * TWO15;
    let b = b * THIRD_V;
    w + (b + a)
}

/// A one-pole decay coefficient for a time `t` (s): `clamp01(E((inv·−ln2)/(t + 1e-30)
/// · 86.5617 + 1200))`, `inv = 1/sr`: the value falls by half every `t`.
#[inline]
fn decay_coef(inv: f32, t: f32) -> f32 {
    c01(exp_e(((inv * LN2N) / (t + EPS30)).mul_add(K865, 1200.0)))
}

/// `E(x·9.965784 + 602.0529)`.
#[inline]
fn db(x: f32) -> f32 {
    exp_e(x.mul_add(DB_K, DB_0))
}

/// `FRECPE`: ARM `FPRecipEstimate` for `f32` (round to nearest, no flush-to-zero).
fn recpe(x: f32) -> f32 {
    let bits = x.to_bits();
    let sign = bits & 0x8000_0000;
    let exp = ((bits >> 23) & 0xff) as i32;
    if x.is_nan() {
        return f32::from_bits(bits | 0x40_0000);
    }
    if exp == 0xff {
        return f32::from_bits(sign);
    }
    if x.abs() < f32::from_bits(0x0020_0000) {
        // Zero or |x| < 2^-128: overflows to infinity.
        return f32::from_bits(sign | 0x7f80_0000);
    }
    // The pseudo-code's 52-bit fraction.
    let mut f: u64 = ((bits & 0x7f_ffff) as u64) << 29;
    let mut e = exp;
    if e == 0 {
        if f & (1 << 51) == 0 {
            e = -1;
            f = (f << 2) & ((1 << 52) - 1);
        } else {
            f = (f << 1) & ((1 << 52) - 1);
        }
    }
    let scaled = 0x100 | (f >> 44) as u32;
    let mut re = 253 - e;
    // RecipEstimate(scaled): 256..511.
    let a = scaled * 2 + 1;
    let est = ((1u32 << 19) / a).div_ceil(2);
    let mut rf: u64 = ((est & 0xff) as u64) << 44;
    if re == 0 {
        rf = (1 << 51) | (rf >> 1);
    } else if re == -1 {
        rf = (1 << 50) | (rf >> 2);
        re = 0;
    }
    f32::from_bits(sign | ((re as u32) << 23) | (rf >> 29) as u32)
}

/// `FRECPS`: `2 − a·b`, fused (∞·0 gives 2).
#[inline]
fn recps(a: f32, b: f32) -> f32 {
    if (a.is_infinite() && b == 0.0) || (a == 0.0 && b.is_infinite()) {
        return 2.0;
    }
    (-a).mul_add(b, 2.0)
}

/// `1/x` as the vector code computes it: estimate, then two Newton steps
/// (`frecpe`, `frecps`, `fmul`, `frecps`, `fmul`).
#[inline]
fn recip(x: f32) -> f32 {
    let e = recpe(x);
    let e = e * recps(x, e);
    recps(x, e) * e
}

/// The vector `2^x` approximation: `x` rounds to an integer `n` (by magnitude, halves
/// up), `2^n` goes into the exponent, `2^(x−n) ≈ 1 + f(0.693 + f(0.242 + 0.056 f))`,
/// each step rounded on its own.
fn exp2_approx(x: f32) -> f32 {
    let a = x.abs();
    let lim = c(0x4b80_0000); // 2^24
    let t = if lim > a { (a as i32) as f32 } else { a };
    let r = if a > t { t + 1.0 } else { t };
    let r = if (r + -0.5) > a { r + -1.0 } else { r };
    let r = f32::from_bits(r.abs().to_bits() | (x.to_bits() & 0x8000_0000));
    let r = if a > lim { x } else { r };
    let n = r as i32;
    let f = x - n as f32;
    let p = f32::from_bits(((n as u32) << 23).wrapping_add(0x3f80_0000));
    let q = f * c(0x3d65_6042) + c(0x3e77_ced9);
    let q = f * q + c(0x3f31_6873);
    (f * q + 1.0) * p
}

/// The vector `log2` approximation of a positive `x`: exponent plus `m(1.3475 −
/// 0.34671 m)` for the mantissa `1 + m`.
fn log2_approx(x: f32) -> f32 {
    let b = x.to_bits();
    let m = f32::from_bits((b & 0x807f_ffff) | 0x3f80_0000) + -1.0;
    let m = m * (c(0x3fac_51ec) - m * c(0x3eb1_4749));
    let e = (((b >> 23) & !0x100) as i32).wrapping_add(-127) as f32;
    m + e
}

/// The parabolic S-shape of the envelope's "shape" curves: `2(x' − x'|x'|) + 0.5`,
/// `x' = x − 0.5`.
#[inline]
fn shape(x: f64) -> f64 {
    let a = x + -0.5;
    (-a).mul_add(a.abs(), a).mul_add(2.0, 0.5)
}

/// `MASCOMMON::SAMPLER::Glide`: the held notes, oldest first (at most 128), and the
/// note a phrase started on.
struct Glide {
    held: Vec<u8>,
    base: u8,
}

impl Glide {
    fn pressed(&self) -> bool {
        !self.held.is_empty()
    }

    /// `onNotePress`: a held note moves to the end; the first note sets the base.
    fn press(&mut self, n: u8) {
        if self.held.is_empty() {
            self.base = n;
        } else if let Some(i) = self.held.iter().rposition(|&x| x == n) {
            self.held.remove(i);
        }
        if self.held.len() < 128 {
            self.held.push(n);
        }
    }

    /// `onNoteRelease`: every entry of the note goes.
    fn release(&mut self, n: u8) {
        self.held.retain(|&x| x != n);
    }
}

/// `TomFBMatrixCore`: its state as a byte image at Maschine's offsets.
struct Core {
    m: Box<[u8; SIZE]>,
    glide: Glide,
}

/// The seven exponential decays: their time slots (+0, value +4, coefficient +8,
/// 1/sr +0xc, floor +0x10).
const DECAYS: [usize; 7] = [0x2e0, 0x2f4, 0x308, 0x31c, 0x330, 0x344, 0x358];

impl Core {
    #[inline]
    fn f(&self, o: usize) -> f32 {
        f32::from_le_bytes(self.m[o..o + 4].try_into().unwrap())
    }
    #[inline]
    fn sf(&mut self, o: usize, v: f32) {
        self.m[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    #[inline]
    fn d(&self, o: usize) -> f64 {
        f64::from_le_bytes(self.m[o..o + 8].try_into().unwrap())
    }
    #[inline]
    fn sd(&mut self, o: usize, v: f64) {
        self.m[o..o + 8].copy_from_slice(&v.to_le_bytes());
    }
    #[inline]
    fn w(&self, o: usize) -> u32 {
        u32::from_le_bytes(self.m[o..o + 4].try_into().unwrap())
    }
    #[inline]
    fn sw(&mut self, o: usize, v: u32) {
        self.m[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn sfs(&mut self, o: usize, v: &[f32]) {
        for (i, x) in v.iter().enumerate() {
            self.sf(o + 4 * i, *x);
        }
    }
    fn zero(&mut self, o: usize, n: usize) {
        self.m[o..o + n].fill(0);
    }

    /// `TomFBMatrixCore()`.
    fn new() -> Core {
        let mut s = Core { m: Box::new([0; SIZE]), glide: Glide { held: Vec::with_capacity(128), base: 0 } };
        s.sfs(0xe0, &[c(0x3000_009d), 1.0]); // noise scale 4.6567e-10 ≈ 2^-31
        s.sf(0xf0, 1.0);
        s.sfs(0x10c, &[1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0]);
        s.sfs(0x150, &[1.0; 4]);
        s.sfs(0x180, &[1.0; 4]);
        s.sfs(0x200, &[1.0; 4]);
        s.sfs(0x210, &[1.0; 4]);
        s.sfs(0x220, &[1.0, 1.0]);
        s.sfs(0x2b8, &[1.0, 1.0]);
        s.sfs(0x2a8, &[0.5, 1.0, 0.75, 1.0]);
        s.sfs(0x2d8, &[1.0, 1.0, 1.0, 0.0]);
        for o in [0x2ec, 0x300, 0x314, 0x328, 0x33c, 0x350] {
            s.sfs(o, &[1.0, c(0x38d1_b717), 1.0, 0.0]);
        }
        // Every coefficient starts as E(1140) = 0.5.
        for o in DECAYS {
            s.sf(o + 8, 0.5);
        }
        s.sfs(0x364, &[1.0, c(0x38d1_b717)]);
        s.sd(0x388, 1.0);
        s.sd(0x3c8, 1.0);
        for o in [0x3d0, 0x3d8, 0x3e0, 0x3e8, 0x3f0, 0x3f8] {
            s.sd(o, 1.0);
        }
        s.sfs(0x420, &[20000.0, 1.0, 1.0, 1.0]);
        s.sfs(0x448, &[1.0, 1.0, 0.0, 0.0]);
        s.sfs(0x458, &[0.0, -1.0]);
        s.sd(0x460, 1.0);
        s.sfs(0x488, &[1.0, 1.0]);
        s.sfs(0x4a8, &[0.5, 0.5, 1.0, 1.0, 1.0]);
        s.sfs(0x4d4, &[0.5, 0.5, 1.0]);
        s.sfs(0x4e4, &[1.0, 1.0, 0.0, c(0x3f00_0001), 1.0]);
        s.sfs(0x634, &[-1.0]);
        s.sw(0x638, u32::MAX);
        s.m[0x600] = 1;
        s.sw(0x610, 1);
        s.sfs(0x280, &[0.0, 1.0, 1.0, 1.0]);
        // The four filters: damping 1.2, h = 1/(g(k + g) + 1) at g = 0.
        s.sfs(0x1f0, &[c(0x3f99_999a); 4]);
        let h = recip(0.0 * (0.0 + c(0x3f99_999a)) + 1.0);
        s.sfs(0x210, &[h; 4]);
        // The noise band-pass: damping 2.
        s.sf(0x2b0, 0.0);
        s.sf(0x2a8, 2.0);
        let g = s.f(0x2a4);
        s.sf(0x2b4, 1.0 / g.mul_add(g + 2.0, 1.0));
        s.sd(0x400, 0.0);
        s.sd(0x408, f64::from_bits(0x4000_cccc_c000_0000));
        s.sd(0x3b0, (LN8N / (s.d(0x3d8) * s.f(0x45c) as f64)).exp());
        s.sd(0x3e8, s.d(0x3f0) + s.d(0x3f0) + E20);
        s.sd(0x3f8, 1.0 / s.d(0x3e8));
        s.sfs(0x424, &[0.5, 0.0]);
        // The clip limits and shapes: bend envelope ±1 (shape 2), fractal ±2.4 (5/6).
        s.sfs(0x468, &[0.5, 0.5, 1.0, 2.0, c(0x3f99_999a), c(0x3f99_999a), c(0x4019_999a), c(0x3f55_5555)]);
        s
    }

    /// `onSetSampleRate(sr, block)`.
    fn set_sample_rate(&mut self, sr: f32, block: i32) {
        // The smoother runs over ≈20 ms of control blocks.
        let p = (((sr * c(0x3ca3_d70a)) / block as f32 + 0.5) as i32).max(1);
        self.sw(0x608, p as u32);
        self.sf(0x60c, 1.0 / (p.wrapping_mul(block)) as f32);
        self.sf(0xe4, sr);
        let inv = 1.0 / sr;
        self.sf(0x110, inv);
        // A 15 Hz one-pole (+0x100; unused by the audio path).
        let f = self.f(0x10c);
        let set15 = if f == -1.0 {
            self.zero(0xf4, 0xc);
            true
        } else {
            let t = tan_s(inv * (f * PI));
            self.sf(0x108, t);
            self.sf(0x100, 1.0 / (t + 1.0));
            self.sf(0x104, t + -1.0);
            self.zero(0xf8, 8);
            f != 15.0
        };
        if set15 {
            self.sf(0x10c, 15.0);
            let t = tan_s(inv * c(0x423c_7edd));
            self.sf(0x108, t);
            self.sf(0x100, 1.0 / (t + 1.0));
            self.sf(0x104, t + -1.0);
        }
        self.zero(0xf4, 0xc);
        self.m[0xdc] = 1;
        let r = (sr / 44100.0).sqrt();
        self.sf(0xe0, r * c(0x3000_009d));
        self.sf(0xf0, 1.0 / r);
        self.sf(0x120, inv);
        self.sf(0x11c, clampf(inv * self.f(0x118), -1.0, 1.0));
        for i in 0..4 {
            let v = clampf(self.f(0x150 + 4 * i) * inv, -1.0, 1.0);
            self.sf(0x170 + 4 * i, v);
            self.sf(0x180 + 4 * i, inv);
        }
        self.sf(0x220, sr);
        self.sf(0x224, inv);
        let top = sr * NYQ;
        self.zero(0x190, 0x50);
        let r12 = recip(12.0);
        for i in 0..4 {
            let fr = exp2_approx(self.f(0x250 + 4 * i) * r12 + LOG2_C).min(top).max(0.0);
            let g = tan_v((fr * PI) * inv);
            let h = recip(g * (self.f(0x1f0 + 4 * i) + g) + 1.0);
            self.sf(0x1e0 + 4 * i, g);
            self.sf(0x200 + 4 * i, fr);
            self.sf(0x210 + 4 * i, h);
        }
        self.sf(0x2b8, sr);
        self.sf(0x2bc, inv);
        // The noise band-pass (+0x290) and the output high-pass (+0x2c0, 10 Hz).
        let f = self.f(0x2ac);
        self.sf(0x2ac, -1.0);
        if f != -1.0 {
            let fl = clampf(f, 0.0, top);
            self.sf(0x2ac, fl);
            let t = tan_s(inv * (fl * PI));
            self.sf(0x2a4, t);
            let k = t + self.f(0x2a8);
            self.sf(0x2b4, 1.0 / t.mul_add(k, 1.0));
        }
        self.zero(0x290, 0x14);
        self.sf(0x2dc, inv);
        let hp = self.f(0x2d8);
        let set10 = if hp == -1.0 {
            true
        } else {
            let t = tan_s(inv * (hp * PI));
            self.sf(0x2d4, t);
            self.sf(0x2cc, 1.0 / (t + 1.0));
            self.sf(0x2d0, t + -1.0);
            hp != 10.0
        };
        self.zero(0x2c0, 0xc);
        if set10 {
            self.sf(0x2d8, 10.0);
            let t = tan_s(inv * c(0x41fb_53d2));
            self.sf(0x2d4, t);
            self.sf(0x2cc, 1.0 / (t + 1.0));
            self.sf(0x2d0, t + -1.0);
        }
        // The decays: value reset, coefficient for the stored time.
        let s9 = inv * LN2N;
        for o in DECAYS {
            self.sf(o + 0xc, inv);
            self.sf(o + 4, 0.0);
            let k = c01(exp_e((s9 / (self.f(o) + EPS30)).mul_add(K865, 1200.0)));
            self.sf(o + 8, k);
            if o == 0x344 && self.f(0x344) != c(0x3b03_126f) {
                // The AM envelope always decays in 2 ms.
                self.sf(0x344, c(0x3b03_126f));
                self.sf(0x34c, c01(exp_e((s9 / c(0x3b03_126f)).mul_add(K865, 1200.0))));
            }
        }
        self.env_set_sample_rate(sr);
        // The click envelope: attack 0 (one sample), decay 70 ms, release 0.
        if self.f(0x45c) > 0.0 && self.f(0x450) != 0.0 {
            self.sf(0x450, 0.0);
            let a = (self.d(0x460) as f32).max(0.0);
            let a = if a > 100.0 { 100.0 } else { a };
            self.sd(0x3c8, a as f64);
            self.sd(0x3d0, 1.0 / a as f64);
        }
        if self.f(0x454) != c(0x3d8f_5c29) {
            self.sf(0x454, c(0x3d8f_5c29));
            let d10 = self.d(0x460);
            let s0 = (d10 as f32).max(c(0x3d8f_5c29));
            let s0 = if s0 > 100.0 { 100.0 } else { s0 };
            let d11 = s0 as f64;
            self.sd(0x3d8, d11);
            let d12 = 1.0 / d11;
            self.sd(0x3e0, d12);
            self.sd(0x3b0, (LN8N / (self.f(0x45c) as f64 * d11)).exp());
            if self.w(0x380) == 2 {
                self.sd(0x3a0, d10 * d12);
                self.sd(0x3a8, 1.0);
                self.sd(0x398, if self.f(0x418) == 0.0 { BIG } else { d11 });
            }
        }
        if self.f(0x458) != 0.0 {
            self.sf(0x458, 0.0);
            self.sd(0x3f0, 0.0);
            let d0 = self.d(0x410);
            if d0 == -1.0 {
                let e = (LN8N / (self.f(0x45c) as f64 * 0.0)).exp();
                self.sd(0x3b8, e);
                if self.w(0x380) == 4 {
                    self.sd(0x3a8, e);
                }
            } else {
                self.sd(0x3b8, 0.0);
                let x = (-d0).mul_add(d0, 1.0);
                let x = 0.0 / (x * x) + E20;
                self.sd(0x3e8, x);
                let y = 1.0 / x;
                self.sd(0x3f8, y);
                if self.w(0x380) == 4 {
                    self.sd(0x3a0, y * self.d(0x460));
                }
            }
        }
        // The two de-click cross-fades (+0x488, +0x4b4) and the choke fade (+0x4e0).
        self.sf(0x4b0, inv);
        let a = inv * self.f(0x488);
        self.sf(0x48c, 1.0);
        self.zero(0x490, 0x18);
        self.sf(0x4a8, 2.0);
        self.sf(0x4ac, a + a);
        self.sf(0x4dc, inv);
        let a = inv * self.f(0x4b4);
        self.sf(0x4b8, 1.0);
        self.zero(0x4bc, 0x18);
        self.sf(0x4d4, 2.0);
        self.sf(0x4d8, a + a);
        self.sf(0x4f4, inv);
        self.sw(0x4e0, 0);
        self.sfs(0x4e8, &[c(0x3ba3_d70a), 0.0]);
        let e = (s9 / c(0x3ba3_d70a)).exp();
        self.sf(0x4f0, if e > 1.0 { 1.0 } else { e });
        self.sf(0x634, sr);
        self.sw(0x638, block as u32);
    }

    /// `onReset()`.
    fn reset(&mut self) {
        self.sw(0x628, 0);
        self.m[0x630] = 0;
        self.zero(0xcc, 0x10);
        self.smoother_jump();
        self.dirty();
        self.sw(0x114, 0);
        self.sw(0x124, 0);
        self.zero(0x130, 0x10);
        self.zero(0x190, 0x50);
        self.zero(0x290, 0x14);
        self.zero(0x2c0, 0xc);
        for o in DECAYS {
            self.sw(o + 4, 0);
        }
        self.zero(0x378, 0x20);
        self.sd(0x388, 1.0);
        self.zero(0x430, 0xc);
        self.sf(0x48c, 1.0);
        self.zero(0x490, 0x18);
        self.sf(0x4b8, 1.0);
        self.zero(0x4bc, 0x18);
        self.sw(0x4e0, 0);
        self.sw(0x4ec, 0);
    }

    /// The smoothed values (+0x510…) jump to their targets (+0x5b0…).
    fn smoother_jump(&mut self) {
        self.m.copy_within(0x5b0..0x600, 0x510);
    }

    /// Mark the smoother dirty (every parameter that feeds it does).
    fn dirty(&mut self) {
        self.m[0x600] = 1;
        self.sw(0x610, 1);
    }

    /// `SmoothingToolbox<18>::startSmooth`: 20 lanes ramp from +0x510 to the targets at
    /// +0x5b0 in steps (+0x560) over the smoothing period; negligible steps jump.
    fn start_smooth(&mut self) {
        let k = self.f(0x60c);
        let mut sum = 0.0f32;
        for i in 0..20 {
            let step = (self.f(0x5b0 + 4 * i) - self.f(0x510 + 4 * i)) * k;
            let sq = step * step;
            if (sq as f64) < 1e-15 {
                self.sf(0x560 + 4 * i, 0.0);
                let t = self.f(0x5b0 + 4 * i);
                self.sf(0x510 + 4 * i, t);
            } else {
                self.sf(0x560 + 4 * i, step);
                sum += sq;
            }
        }
        if sum > 0.0 {
            let p = self.w(0x608);
            self.sw(0x604, p);
            self.m[0x601] = 1;
        } else {
            self.sw(0x610, 4);
        }
    }

    /// `processControlRate()`: starts, counts down and ends the parameter ramps.
    fn control_rate(&mut self) {
        if self.m[0x600] != 0 {
            self.m[0x600] = 0;
            self.sw(0x610, 2);
            self.start_smooth();
            return;
        }
        let n = self.w(0x604).wrapping_sub(1);
        self.sw(0x604, n);
        if n == 0 {
            self.smoother_jump();
            self.zero(0x560, 0x50);
            self.m[0x601] = 0;
            self.sw(0x610, 4);
        } else if self.w(0x610) == 2 {
            self.sw(0x610, 3);
        } else if self.w(0x610) == 4 {
            self.sw(0x610, 0);
        }
    }

    /// `setParameter(idx, v)`.
    fn set_parameter(&mut self, idx: u32, v: f32) {
        if idx > 12 {
            return;
        }
        self.sf(0x80 + 4 * idx as usize, v);
        match idx {
            0 => {
                // Glide: E(40v dB)·1000·0.001 s, 1e-4 at 0.
                let g = if v <= 0.0 { c(0x38d1_b717) } else { (db(c01(v).mul_add(40.0, 0.0)) * 1000.0) * c(0x3a83_126f) };
                self.sf(0x5e0, g);
                self.dirty();
            }
            1 => self.sf(0xc8, c01(v).mul_add(36.0, -18.0)),
            2 => self.m[0xb4] = u8::from(v > 0.0),
            3 => self.set_decay(v),
            4 => {
                self.sf(0x5c8, c01(v).mul_add(48.0, -24.0));
                self.dirty();
            }
            5 => {
                self.sf(0x5dc, v.abs().mul_add(c(0xbf4c_cccd), c(0x3fe6_6666)) * v);
                self.dirty();
            }
            6 => {
                let x = c01(v);
                self.sf(0x5e8, exp_e(x.mul_add(5.0, -55.0).mul_add(DB_K, 1200.0)));
                self.sf(0x5ec, x.mul_add(2.0, 0.0));
                let y = c01(v / c(0x3e4c_cccd));
                self.sf(0x5f0, y.mul_add(0.5, c(0x3e4c_cccd)));
                self.sf(0x5f4, x.mul_add(15.0, 0.0));
                self.dirty();
            }
            7 => {
                self.sf(0x5b8, c01(v).mul_add(24.0, -12.0));
                self.dirty();
            }
            8 => {
                self.sf(0x5bc, c01(v).mul_add(30.0, 40.0) + -48.0);
                self.dirty();
            }
            9 => {
                let x = c01(v).mul_add(90.0, 30.0) + -48.0;
                self.sfs(0x5c0, &[x, x + -5.0]);
                self.dirty();
            }
            10 => {
                self.sf(0x5b0, v);
                self.dirty();
            }
            11 => {
                self.sf(0x5b4, v);
                self.dirty();
            }
            _ => {
                self.sf(0x5e4, v);
                self.dirty();
            }
        }
    }

    /// `setParamDecay(v)`: `x = 40 + 25v`; the decay times (as E of dB values) of the
    /// pitch sweep (x−5), oscillator level (0.74x), feedback level (0.74x−3) and
    /// tone body (max(x−28, 28)).
    fn set_decay(&mut self, v: f32) {
        let x = c01(v).mul_add(25.0, 40.0);
        let b = x * c(0x3f3d_70a4);
        let m = x + -28.0;
        let m = if m < 28.0 { 28.0 } else { m };
        self.sf(0x5cc, db(x + -5.0));
        self.sf(0x5d0, db(b));
        self.sf(0x5d4, db(b + -3.0));
        self.sf(0x5d8, db(m));
        self.dirty();
    }

    /// `setNoteOn(vel, note)`.
    fn note_on(&mut self, vel: f32, note: u32) {
        self.sw(0x62c, (note as i32 - 60) as u32);
        let legato = self.glide.pressed();
        self.glide.press(note as u8);
        if !(0.0..=1.0).contains(&vel) {
            self.sw(0x628, 0);
            return;
        }
        let am = self.f(0x514);
        self.smoother_jump();
        self.dirty();
        let t = self.f(0xc8);
        self.sf(0xc4, t);
        if !legato {
            let am = c01(am);
            let v = c01(vel).mul_add(3.0, 7.0) * 4.0;
            let imp = c01(self.f(0x98));
            self.sf(0xb8, (v + -2.0).mul_add(imp, 2.0));
            self.sf(0x114, 0.25);
            self.zero(0x130, 0x10);
            self.zero(0x190, 0x50);
            for o in DECAYS {
                self.sf(o + 4, 1.0);
            }
            let s9 = am.mul_add(c(0x458f_c000), 400.0);
            self.env_note_on(vel);
            let a = self.f(0xcc);
            self.sf(0xd0, if a > vel { a } else { vel });
            let a = self.f(0xd4);
            self.sf(0xd8, if a > vel { a } else { vel });
            // De-click fades from the previous note's output.
            let x = self.f(0xc0);
            self.sf(0x488, x);
            self.sf(0x4ac, (x * self.f(0x4b0)) * self.f(0x4a8));
            self.sfs(0x48c, &[1.0, 0.0]);
            self.sf(0x4a0, self.f(0x498) - self.f(0x49c));
            self.sf(0x4a4, self.f(0x494));
            self.sf(0x4b4, s9);
            self.sf(0x4d8, (s9 * self.f(0x4dc)) * self.f(0x4d4));
            self.sfs(0x4b8, &[1.0, 0.0]);
            self.sf(0x4cc, self.f(0x4c4) - self.f(0x4c8));
            self.sf(0x4d0, self.f(0x4c0));
        }
        self.sw(0x4e0, 1);
        self.sw(0x4ec, 0);
        self.sf(0x628, vel);
        self.m[0x630] = 1;
    }

    /// `onChoke()`: a 5 ms fade.
    fn choke(&mut self) {
        if self.w(0x4e0) == 1 {
            self.sw(0x4e0, 2);
            self.sf(0x4ec, 1.0);
        }
    }

    /// `hasOutputSignal()`.
    fn has_output(&self) -> bool {
        (self.f(0x320) >= self.f(0x32c) || self.f(0x334) >= self.f(0x340) || self.w(0x380) != 0) && self.w(0x4e0) != 0
    }

    // `FreeEnvelope<off, breakpoint, lin, shape, off>` at +0x370: the click envelope.

    /// `FreeEnvelope::setSamplerate(sr)`.
    fn env_set_sample_rate(&mut self, sr: f32) {
        const E: usize = 0x370;
        if sr <= 0.0 {
            return;
        }
        self.sf(E + 0xec, sr);
        let d8 = sr as f64;
        let d2 = 1.0 / d8;
        self.sd(E + 0xf0, d2);
        let s5 = 1.0 / sr;
        self.sf(E + 0xdc, s5);
        let s4 = self.f(E + 0xd8);
        self.sf(E + 0xd8, -1.0);
        if s4 != -1.0 {
            let w = tan_s(s5 * (s4 * PI));
            self.sfs(E + 0xd4, &[w, s4]);
            self.sfs(E + 0xcc, &[1.0 / (w + 1.0), w + -1.0]);
        }
        self.zero(E + 8, 0x10);
        self.sd(E + 0x18, 1.0);
        self.sd(E + 0x20, 0.0);
        self.zero(E + 0xc0, 0xc);
        let s0 = self.d(E + 0x58) as f32;
        let s2 = d2 as f32;
        let s3 = if s2 > s0 { s2 } else { s0 };
        let s3 = if s3 > 100.0 { 100.0 } else { s3 };
        self.sd(E + 0x58, s3 as f64);
        self.sd(E + 0x60, 1.0 / s3 as f64);
        let s3 = self.d(E + 0x68) as f32;
        self.sfs(E + 0xe0, &[s0, s3]);
        let s0 = if s2 > s3 { s2 } else { s3 };
        let s0 = if s0 > 100.0 { 100.0 } else { s0 };
        let d0 = s0 as f64;
        self.sd(E + 0x68, d0);
        self.sd(E + 0x70, 1.0 / d0);
        self.sd(E + 0x40, (LN8N / (d8 * d0)).exp());
        let s0 = self.d(E + 0x78) as f32;
        self.sf(E + 0xe8, s0);
        let d0 = s0 as f64;
        self.sd(E + 0x80, d0);
        let d1 = self.d(E + 0xa0);
        if d1 == -1.0 {
            self.sd(E + 0x48, (LN8N / (d8 * d0)).exp());
        } else {
            self.sd(E + 0x48, 0.0);
            let d1 = (-d1).mul_add(d1, 1.0);
            let x = (d0 + d0) / (d1 * d1) + E20;
            self.sd(E + 0x78, x);
            self.sd(E + 0x88, 1.0 / x);
        }
    }

    /// `FreeEnvelope::setNoteOn(vel)`.
    fn env_note_on(&mut self, vel: f32) {
        const E: usize = 0x370;
        if !(vel >= 0.0 && vel < c(0x3f80_0347)) {
            return;
        }
        self.zero(E + 8, 0x10);
        self.env_advance();
        let k = self.f(E + 0xb8);
        self.sf(E + 0xbc, k.mul_add(vel * vel, 1.0 - k));
    }

    /// `FreeEnvelope::advanceStage()`: idle → attack → decay → release → idle.
    fn env_advance(&mut self) {
        const E: usize = 0x370;
        self.sd(E + 0x18, 1.0);
        self.sd(E + 0x20, 0.0);
        match self.w(E + 0x10) {
            0 => {
                let d2 = self.d(E + 0x58);
                let d0 = self.d(E + 0xf0) * self.d(E + 0x60);
                self.sd(E + 0x38, 1.0);
                self.sw(E + 0x10, 1);
                self.sd(E + 0x28, d2);
                self.sd(E + 0x30, d0);
            }
            1 => {
                self.sd(E + 0x30, self.d(E + 0xf0) * self.d(E + 0x70));
                self.sd(E + 0x38, 1.0);
                self.sw(E + 0x10, 2);
                let t = if self.f(E + 0xa8) == 0.0 { BIG } else { self.d(E + 0x68) };
                self.sd(E + 0x28, t);
            }
            2 | 3 => {
                let v = self.f(E + 0x14);
                self.sf(E + 0x50, v);
                if self.d(E + 0xa0) == -1.0 {
                    self.sd(E + 0x30, 0.0);
                    self.sd(E + 0x38, self.d(E + 0x48));
                } else {
                    self.sd(E + 0x30, self.d(E + 0xf0) * self.d(E + 0x88));
                    self.sd(E + 0x38, 1.0);
                }
                self.sd(E + 0x28, BIG);
                self.sw(E + 0x10, 4);
            }
            4 => {
                self.sd(E + 0x30, 0.0);
                self.sd(E + 0x38, 1.0);
                self.zero(E + 0x10, 8);
                self.sd(E + 0x18, 1.0);
            }
            _ => {}
        }
    }

    /// A curved stage: `(m + 1) + (m + 1)/(k − (x + m + 1)·k)` with `k, m` from the
    /// curvature `|d3|` (≤ 0.999).
    fn env_curve(x: f64, d3: f64) -> f64 {
        let lim = f64::from_bits(0x3fef_f7ce_d916_872b);
        let d5 = d3.abs();
        let d5 = if d5 > lim { lim } else { d5 };
        let a = (d5 + 1.0) * (d5 + 1.0) + -1.0;
        let a_inv = 1.0 / a;
        let b = 1.0 - d5;
        let b2 = b * b;
        let bi = 1.0 / b;
        let bi2 = bi * bi;
        let (k, m) = if d5 <= 0.0 { (a, a_inv) } else { (bi2, b2) };
        let d4 = m + 1.0;
        let y = (-(x + d4)).mul_add(k, k);
        d4 + d4 / y
    }

    /// One sample of the envelope (`processSampleStage<stage>()`).
    fn env_sample(&mut self) -> f32 {
        const E: usize = 0x370;
        let st = self.w(E + 0x10);
        if st == 1 || st == 2 || st == 4 {
            let ph = clamp01d((self.d(E + 0x18) - self.d(E + 0x30)) * self.d(E + 0x38));
            let time = self.d(E + 0xf0) + self.d(E + 0x20);
            self.sd(E + 0x18, ph);
            self.sd(E + 0x20, time);
            let out = match st {
                1 => {
                    let mut x = 1.0 - ph;
                    let d3 = self.d(E + 0x90);
                    if d3 != 0.0 {
                        x = Self::env_curve(x, d3);
                    }
                    if d3 > 0.0 {
                        x = shape(x);
                    }
                    x as f32
                }
                2 => {
                    let y = ph + -MIN4;
                    let z = y * f64::from_bits(0xbff0_0068_de3a_efe5) + 1.0;
                    let mut x = if y < 0.0 { 1.0 } else { z };
                    let d3 = self.d(E + 0x98);
                    if d3 != 0.0 {
                        x = Self::env_curve(x, d3);
                    }
                    if d3 > 0.0 {
                        x = shape(x);
                    }
                    let s = self.f(E + 0xa8) as f64;
                    (1.0 - s).mul_add(1.0 - x, s) as f32
                }
                _ => {
                    let y = ph + -MIN4;
                    let z = y * f64::from_bits(0x3ff0_0068_de3a_efe5);
                    let x = if y < 0.0 { 0.0 } else { z };
                    let x = if self.d(E + 0xa0) == -1.0 { x } else { (x + -1.0) + 1.0 };
                    (x * self.f(E + 0x50) as f64) as f32
                }
            };
            self.sf(E + 0x14, out);
            if ph <= MIN4 || time > self.d(E + 0x28) {
                self.env_advance();
            }
        }
        self.f(E + 0x14) * (self.f(E + 0xbc) * self.f(E + 0xb4))
    }

    /// One of the seven exponential decays: a new time `t` recomputes the coefficient;
    /// the value is multiplied by it and returned minus the floor (0, and the value
    /// cleared, once below it).
    #[inline]
    fn decay(&mut self, o: usize, t: Option<f32>) -> f32 {
        if let Some(t) = t
            && self.f(o) != t
        {
            self.sf(o, t);
            let k = decay_coef(self.f(o + 0xc), t);
            self.sf(o + 8, k);
        }
        let v = self.f(o + 8) * self.f(o + 4);
        self.sf(o + 4, v);
        let fl = self.f(o + 0x10);
        if v < fl {
            self.sf(o + 4, 0.0);
            0.0
        } else {
            v - fl
        }
    }

    /// `processAudio` for `l.len()` frames.
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.f(0x634) <= 0.0 {
            return;
        }
        if !self.has_output() {
            if self.m[0x630] != 0 {
                self.reset();
            }
            return;
        }
        const FMK: [f32; 4] = [2.25, 0.0, 1.25, c(0x3fa3_3333)];
        const AMK: [f32; 4] = [c(0x3f82_8f5c), 0.0, c(0x3f8a_3d71), c(0x3f97_0a3d)];
        for (ol, or) in l.iter_mut().zip(r.iter_mut()) {
            // Parameter ramps.
            for i in 0..20 {
                let v = self.f(0x510 + 4 * i) + self.f(0x560 + 4 * i);
                self.sf(0x510 + 4 * i, v);
            }
            // The note in semitones (tune + note − 60) and its frequency.
            let s11 = self.f(0xc4) + self.w(0x62c) as i32 as f32;
            let hz = exp_e(s11.mul_add(5.0, PITCH0));
            // The pitch-sweep decay (lane +0x52c) and the bend envelope.
            let e1 = self.decay(0x2e0, Some(self.f(0x52c)));
            let s8 = s11 + c(0x4243_999a);
            let t2 = db(s8.mul_add(-0.75, 44.0));
            let lim = self.f(0x470);
            let e2 = clampf(self.decay(0x2f4, Some(t2)), -lim, lim);
            let shape2 = self.f(0x474);
            // Glide: a one-pole on the pitch.
            let glide = self.f(0x540);
            let mut s14 = s8;
            if glide > 0.0 {
                let x = exp_e((LN2N / (glide * self.f(0x634))).mul_add(K865, 1200.0));
                let x = if x < 0.0 { 0.0 } else { x };
                let k = if x > 1.0 { 0.0 } else { 1.0 - x };
                let p = self.f(0xbc);
                s14 = (s8 - p).mul_add(k, p);
                self.sf(0xbc, s14);
            }
            let bend_amt = (1.0 - e1) * self.f(0x528);
            let s15 = e2 * (e2.abs() * -0.25).mul_add(shape2, 1.0);
            let x = self.f(0xb8).mul_add(s15, bend_amt + s14);
            let s13 = exp_e(x.mul_add(5.0, PITCH0));
            self.sf(0x118, s13);
            self.sf(0x11c, clampf(s13 * self.f(0x120), -1.0, 1.0));
            let vel = self.f(0x628);
            let env = self.env_sample();
            let e4 = self.decay(0x31c, Some(self.f(0x530)));
            let s1 = c(0x4102_d6b6) / hz;
            let s3 = e4 * self.f(0xd0);
            self.sf(0xcc, s3);
            // The classic tom: phase → triangle → sine.
            let p = self.f(0x114) + self.f(0x11c) + 10.0;
            let p = p - ((p + 0.5) as i32) as f32;
            self.sf(0x114, p);
            let q = self.f(0x124) + p + 10.0;
            let q = q - ((q + 0.5) as i32) as f32;
            let tri = (q + q).abs() + -0.5;
            self.sf(0x128, tri);
            let s5 = (tri * PI) * c(0x3e22_f981);
            let h = if s5 < 0.0 { -0.5f64 } else { 0.5 };
            let t7 = s1 * c(0x393c_be62);
            let t7 = if t7 > c(0x3abe_0ded) { c(0x3abe_0ded) } else { t7 };
            let e7 = self.decay(0x358, Some(t7));
            let amp = (vel * self.f(0x550)) * env;
            let n = (h + s5 as f64) as i32;
            let x = ((s5 - n as f32) + (s5 - n as f32)) * PI;
            let x2 = x * x;
            let y = x2.mul_add(c(0xb2cd_4ca8), c(0x3638_b94f));
            let y = y.mul_add(x2, c(0xb950_0c03));
            let y = y.mul_add(x2, c(0x3c08_8885));
            let y = y.mul_add(x2, c(0xbe2a_aac1));
            let y = y.mul_add(x2, 1.0);
            let sine = x * y;
            let k = e7 * 2000.0;
            let k = if k > 1.0 { 1.0 } else { k };
            let k = if k < 0.0 { 0.0 } else { k };
            let k = (self.f(0x554) + -1.0).mul_add(k, 1.0);
            let sine = clampf(sine * k, -1.0, 1.0);
            let sine = sine * (-sine.abs()).mul_add(e4, e4 + 1.0);
            let mut tom = (amp + s3) * sine;
            self.sf(0x49c, self.f(0x498));
            self.sf(0x494, tom);
            if self.f(0x48c) > 0.0 {
                let a = self.f(0x48c) - self.f(0x4ac);
                self.sf(0x48c, a);
                let a = shape32(a);
                let o = self.f(0x490);
                let b = (self.f(0x4a4) + o) * a;
                self.sf(0x490, o + self.f(0x4a0));
                tom += b;
            }
            self.sf(0x498, tom);
            self.sf(0xc0, s13);
            let e5 = self.decay(0x330, Some(self.f(0x534)));
            let s2 = self.f(0xd8) * e5;
            self.sf(0xd4, s2);
            // The four matrix oscillators' pitches (+0x250) and filters.
            let kt = if self.m[0xb4] == 0 { 0.0 } else { s11 * c(0xbe77_ced9) };
            let base = [self.f(0x51c) + kt, 0.0, self.f(0x520), kt + self.f(0x524)];
            let t = s14 + self.f(0x518);
            let b20 = s15 * 20.0;
            let r12 = recip(12.0);
            let top = self.f(0x220) * NYQ;
            let inv = self.f(0x224);
            let mut fr = [0.0f32; 4];
            for i in 0..4 {
                let bend = if i == 3 { 0.0 } else { bend_amt };
                let pch = base[i] + (b20 + (bend + t));
                self.sf(0x250 + 4 * i, pch);
                fr[i] = exp2_approx(r12 * pch + LOG2_C);
                let fc = fr[i].min(top).max(0.0);
                let g = tan_v((fc * PI) * inv);
                let h = recip(g * (self.f(0x1f0 + 4 * i) + g) + 1.0);
                self.sf(0x1e0 + 4 * i, g);
                self.sf(0x200 + 4 * i, fc);
                self.sf(0x210 + 4 * i, h);
            }
            // FM: lane i's pitch moves between 12·log2(1/(1+d)) and 12·log2(1+d)
            // semitones (d = FM·k_i) as its feedback (+0x240) goes from −2 to 2.
            let fm = self.f(0x510);
            let e6 = self.decay(0x344, None);
            let amk = c01(e6.mul_add(-4.0, c(0x3f99_999a))) * self.f(0x514);
            let mut osc = [0.0f32; 4];
            for i in 0..4 {
                let d = FMK[i] * fm;
                self.sf(0x270 + 4 * i, d);
                let up = d + 1.0;
                let a = (log2_approx(recip(up)) + -LOG2_C) * 12.0;
                let b = (log2_approx(up) + -LOG2_C) * 12.0;
                let fb = self.f(0x240 + 4 * i);
                #[allow(clippy::manual_clamp)] // min/max, not clamp: NaN maps to 1, as fminnm/fmaxnm do
                let w = (fb * 0.25 + 0.5).min(1.0).max(0.0);
                let st = a + w * (b - a);
                let mf = fr[i] * exp2_approx(r12 * st + LOG2_C);
                let am_v = AMK[i] * amk;
                self.sf(0x260 + 4 * i, am_v);
                let inc = clampf(self.f(0x180 + 4 * i) * mf, -1.0, 1.0);
                self.sf(0x150 + 4 * i, mf);
                self.sf(0x170 + 4 * i, inc);
                // Phase and the parabolic sine p(8 − 16|p|).
                let ph = self.f(0x130 + 4 * i) + inc + 10.0;
                let x = ph + 0.5;
                let rx = if c(0x4b80_0000) > x.abs() { (x as i32) as f32 } else { x };
                let p = ph - rx;
                self.sfs(0x130 + 4 * i, &[p]);
                self.sf(0x140 + 4 * i, p);
                let par = p * ((ph - rx).abs() * -16.0 + 8.0);
                // AM by the feedback: lane 0 multiplies, the others ring around 1.
                let m = if self.f(0x280 + 4 * i) > 0.0 {
                    fb * am_v
                } else {
                    #[allow(clippy::manual_clamp)] // NaN maps to 1, as fminnm/fmaxnm do
                    let am = am_v.min(1.0).max(0.0);
                    (fb + -1.0) * am
                };
                osc[i] = (m + 1.0) * par;
            }
            // The state-variable filters.
            let mut s1n = [0.0f32; 4];
            for i in 0..4 {
                let g = self.f(0x1e0 + 4 * i);
                let k = self.f(0x1f0 + 4 * i);
                let z1 = self.f(0x190 + 4 * i);
                let z2 = self.f(0x1a0 + 4 * i);
                let a = z1 + z1;
                let b = (z2 + z2) + g * a;
                let gk = g * g + g * k;
                let b = b + a * k;
                let b = b + osc[i] * gk;
                let hp = osc[i] - self.f(0x210 + 4 * i) * b;
                let bp = z1 + g * hp;
                let s1 = z1 + bp;
                let lp = z2 + g * s1;
                let s2 = z2 + lp;
                self.sf(0x1c0 + 4 * i, s1);
                self.sf(0x1d0 + 4 * i, s2);
                self.sf(0x190 + 4 * i, bp);
                self.sf(0x1a0 + 4 * i, lp);
                self.sf(0x1b0 + 4 * i, hp);
                self.sf(0x230 + 4 * i, osc[i]);
                s1n[i] = s1;
            }
            // The fractal voice: lane 0, soft-clipped, crossfaded with its filter by Color.
            let lim = self.f(0x480);
            let x = clampf(0.5 * osc[0], -lim, lim);
            let x = x * (x.abs() * -0.25).mul_add(self.f(0x484), 1.0);
            let color = c01(self.f(0x544));
            let x = (x - s1n[0]).mul_add(color, s1n[0]);
            // The feedback matrix: each lane gets the sum of the others.
            let sum = (s1n[0] + s1n[1]) + (s1n[2] + s1n[3]);
            for (i, s) in s1n.iter().enumerate() {
                self.sf(0x240 + 4 * i, sum - s);
            }
            let mut frac = (amp + s2) * x;
            self.sf(0x4c8, self.f(0x4c4));
            self.sf(0x4c0, frac);
            if self.f(0x4b8) > 0.0 {
                let a = self.f(0x4b8) - self.f(0x4d8);
                self.sf(0x4b8, a);
                let a = shape32(a);
                let o = self.f(0x4bc);
                let b = (self.f(0x4d0) + o) * a;
                self.sf(0x4bc, o + self.f(0x4cc));
                frac += b;
            }
            self.sf(0x4c4, frac);
            // Mix: (1−m)(1+m)·tom + m(2−m)·fractal, then the 10 Hz high-pass.
            let mix = c01(self.f(0x53c));
            let a = 1.0 - mix;
            let a = a * (2.0 - a);
            let b = mix * (2.0 - mix);
            let y = a.mul_add(tom, frac * b);
            let z = self.f(0x2c0);
            let y = (-self.f(0x2d0)).mul_add(z, y);
            let s1 = self.f(0x2cc) * y;
            self.sf(0x2c8, s1 - z);
            self.sf(0x2c0, s1);
            let out = s1 - z;
            // The noise burst (LCG) through a band-pass at the pitch + 40 semitones.
            let seed = self.w(0xe8).wrapping_mul(1_103_515_245).wrapping_add(12345);
            self.sw(0xe8, seed);
            let nf = exp_e(((s8 + 40.0) + bend_amt).mul_add(5.0, PITCH0));
            let (g, k, hh) = if self.f(0x2ac) == nf {
                (self.f(0x2a4), self.f(0x2a8), self.f(0x2b4))
            } else {
                let f = clampf(nf, 0.0, self.f(0x2b8) * NYQ);
                self.sf(0x2ac, f);
                let g = tan_s(self.f(0x2bc) * (f * PI));
                self.sf(0x2a4, g);
                let k = self.f(0x2a8);
                let hh = 1.0 / g.mul_add(k + g, 1.0);
                self.sf(0x2b4, hh);
                (g, k, hh)
            };
            let nz = self.f(0xe0) * seed as i32 as f32;
            let z1 = self.f(0x290);
            let z2 = self.f(0x294);
            let a = z1 + z1;
            let b = z2.mul_add(2.0, g * a);
            let gk = g.mul_add(k, g * g);
            let b = a.mul_add(k, b);
            let b = nz.mul_add(gk, b);
            let hp = nz - hh * b;
            self.sf(0x298, hp);
            let n1 = g.mul_add(hp, z1);
            let bp = z1 + n1;
            self.sf(0x29c, bp);
            let n2 = g.mul_add(bp, z2);
            self.sf(0x2a0, z2 + n2);
            self.sf(0x290, n1);
            self.sf(0x294, n2);
            let e3 = self.decay(0x308, Some(self.f(0x548)));
            let nlev = self.f(0x54c);
            // The choke fade.
            let gain = match self.w(0x4e0) {
                0 => 0.0,
                1 => 1.0,
                _ => {
                    let g = self.f(0x4f0) * self.f(0x4ec);
                    self.sf(0x4ec, g);
                    if (g as f64) < MIN4 {
                        self.sw(0x4ec, 0);
                        self.sw(0x4e0, 0);
                        0.0
                    } else {
                        g
                    }
                }
            };
            let out = ((bp * e3) * nlev).mul_add(vel, out);
            let v = (out * c(0x3ea7_963a)) * gain;
            *ol = v;
            *or = v;
        }
    }
}

/// The de-click fades' curve: `2(x' − x'|x'|) + 0.5` of `x' = x − 0.5` (`f32`).
#[inline]
fn shape32(x: f32) -> f32 {
    let a = x + -0.5;
    (-a).mul_add(a.abs(), a).mul_add(2.0, 0.5)
}

/// Tom's Fractal engine (`TomProcessor` mode 1).
pub struct Fractal {
    c: Core,
}

impl Fractal {
    pub fn new() -> Fractal {
        Fractal { c: Core::new() }
    }
}

impl Engine for Fractal {
    fn set_sample_rate(&mut self, sr: f32) {
        self.c.set_sample_rate(sr, 128);
    }

    fn reset(&mut self) {
        self.c.reset();
    }

    fn set_shared(&mut self, idx: u32, v: f32, _shared: &[f32; 7]) {
        // `TomProcessor::setParamTune/Decay/Bend/Impact` → `setParameter(1/3/4/6)`;
        // Tension does nothing here.
        match idx {
            1 => self.c.set_parameter(1, v),
            2 => self.c.set_parameter(3, v),
            3 => self.c.set_parameter(4, v),
            4 => self.c.set_parameter(6, v),
            _ => {}
        }
    }

    fn update_global(&mut self, shared: &[f32; 7]) {
        self.c.set_parameter(1, shared[1]);
        self.c.set_parameter(3, shared[2]);
        self.c.set_parameter(4, shared[3]);
        self.c.set_parameter(6, shared[4]);
    }

    fn set_core(&mut self, idx: u32, v: f32) {
        self.c.set_parameter(idx, v);
    }

    fn note_on(&mut self, vel: f32, note: u32, shared: &[f32; 7]) {
        // Velocity scaling s (`TomProcessor::setNoteOn`): (1 − |s|(1 − v))³ for s ≥ 0,
        // (1 − |s|·v)³ for s < 0.
        let s = shared[6];
        let x = if s < 0.0 { vel } else { 1.0 - vel };
        let y = (-x).mul_add(s.abs(), 1.0);
        self.c.note_on(y * (y * y), note);
    }

    fn note_off(&mut self, note: u32) {
        self.c.glide.release(note as u8);
    }

    fn choke(&mut self) {
        self.c.choke();
    }

    fn control_rate(&mut self) {
        self.c.control_rate();
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.c.process(l, r);
    }

    fn silent(&self) -> bool {
        !self.c.has_output()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(sr: f32) -> Fractal {
        let mut f = Fractal::new();
        f.set_sample_rate(44100.0);
        let shared = [1.0, 0.0, 0.5, 0.5, 0.8, 1.0, 1.0];
        for (i, v) in [(2, 0.0), (5, 0.05), (0, 0.1), (7, 0.5), (8, 1.0), (9, 0.75), (10, 0.5), (11, 0.5), (12, 0.5)] {
            f.set_core(i, v);
        }
        f.set_sample_rate(sr);
        f.reset();
        f.update_global(&shared);
        f
    }

    fn render(f: &mut Fractal, n: usize) -> Vec<f32> {
        let mut out = Vec::new();
        let (mut l, mut r) = ([0f32; 16], [0f32; 16]);
        let mut ctl = 0;
        for _ in 0..n / 16 {
            if ctl == 0 {
                f.control_rate();
                ctl = 128;
            }
            l.fill(0.0);
            r.fill(0.0);
            f.process(&mut l, &mut r);
            ctl -= 16;
            out.extend_from_slice(&l);
        }
        out
    }

    #[test]
    fn recip_estimate() {
        assert_eq!(recpe(1.0), 0.998_046_9);
        assert_eq!(recpe(12.0), f32::from_bits(0x3daa_8000));
        assert!((recip(12.0) - 1.0 / 12.0).abs() < 1e-8);
    }

    #[test]
    fn approximations() {
        for x in [-3.0f32, -0.4, 0.0, 0.7, 5.2] {
            assert!((exp2_approx(x) / x.exp2() - 1.0).abs() < 1e-3, "{x}");
        }
        for x in [0.3f32, 1.0, 7.5, 300.0] {
            assert!((log2_approx(x) - x.log2()).abs() < 1e-2, "{x}");
        }
    }

    #[test]
    fn note_sounds_then_ends() {
        let mut f = engine(48000.0);
        assert!(f.silent());
        f.note_on(0.8, 60, &[1.0, 0.0, 0.5, 0.5, 0.8, 1.0, 1.0]);
        assert!(!f.silent());
        let out = render(&mut f, 48000 * 3);
        assert!(out.iter().all(|x| x.is_finite()));
        let peak = out.iter().fold(0f32, |a, x| a.max(x.abs()));
        assert!(peak > 0.01, "{peak}");
        assert!(f.silent());
        assert!(out[out.len() - 1000..].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn set_mid_note_keeps_the_voice() {
        let mut f = engine(44100.0);
        f.note_on(1.0, 50, &[1.0, 0.0, 0.5, 0.5, 0.8, 1.0, 1.0]);
        render(&mut f, 1024);
        f.set_core(10, 0.9);
        f.set_shared(2, 0.7, &[1.0, 0.0, 0.7, 0.5, 0.8, 1.0, 1.0]);
        let out = render(&mut f, 512);
        assert!(!f.silent());
        assert!(out.iter().any(|x| x.abs() > 1e-4));
    }

    #[test]
    fn choke_fades_out() {
        let mut f = engine(44100.0);
        f.note_on(1.0, 60, &[1.0, 0.0, 0.5, 0.5, 0.8, 1.0, 1.0]);
        render(&mut f, 256);
        f.choke();
        render(&mut f, 44100 / 2);
        assert!(f.silent());
    }
}
