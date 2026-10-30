//! Small kernels shared by the modal-drum sub-cores: the `ExpTable` with explicit
//! (index, fraction) lookups, and `NI::DSP::UTIL::MemoEnvelope`.

use std::sync::OnceLock;

/// `NI::AB::ExpTable` (docs/fx-eq.md §2.1): `(val, slope)`, `val[i] = 2^(i/60 − 20)`.
fn table() -> &'static [(f32, f32); 2401] {
    static T: OnceLock<Box<[(f32, f32); 2401]>> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = Box::new([(0f32, 0f32); 2401]);
        t[0].0 = f32::from_bits(0x3580_0000);
        for (i, e) in t.iter_mut().enumerate().skip(1) {
            e.0 = (i as f64 / 60.0 + -20.0).exp2() as f32;
        }
        for i in 0..2400 {
            t[i].1 = t[i + 1].0 - t[i].0;
        }
        t
    })
}

/// A lookup the compiler folded to `val[i] + slope[i]·frac` (fused) with a constant
/// index and fraction.
#[inline]
pub(super) fn et(i: usize, frac: f32) -> f32 {
    let (v, s) = table()[i];
    s.mul_add(frac, v)
}

/// `E(x)` as `exp_e` (re-exported for the sub-cores).
#[inline]
pub(super) fn e(x: f32) -> f32 {
    crate::fx::exp_e(x)
}

/// The pitch-unit lookup `E((60 − x)·9.965784 + 1200)` (a rate in Hz for x semitones).
#[inline]
pub(super) fn e_rate(x: f32) -> f32 {
    e((60.0 - x).mul_add(9.965_784, 1200.0))
}

/// The time lookup `E(x·9.965784 + 602.0529)` (seconds).
#[inline]
pub(super) fn e_time(x: f32) -> f32 {
    e(x.mul_add(9.965_784, 602.052_9))
}

/// The velocity curve with two bends and no crossfade (`MainOscAttack`,
/// `ParallelNoise::setNoteOn`): normalise `v` into `[lo, hi]`, bend by `k1`, `k2`.
#[inline]
pub(super) fn vcurve2(v: f32, lo: f32, hi: f32, k1: f32, k2: f32) -> f32 {
    let hi = if hi > lo { hi } else { lo + 0.002 };
    let x = if v < lo { lo } else { v };
    let x = if x > hi { hi } else { x };
    let d = hi - lo;
    let d = if d < 1e-7 { 1e-7 } else { d };
    let y = (x - lo) / d;
    let y = y * (-y.abs()).mul_add(k1, k1 + 1.0);
    let y = y * (-y.abs()).mul_add(k2, k2 + 1.0);
    super::clamp01(y)
}

/// `NI::DSP::UTIL::MemoEnvelope` (0x80 bytes): attack (linear) → hold → decay
/// (linear) → release (exponential); a retrigger continues from the current value.
#[derive(Clone, Copy, Default)]
pub(super) struct MemoEnv {
    /// +0: 0 off, 1 attack, 2 hold, 3 decay, 4 release.
    pub state: u32,
    /// +4 attack, +8 hold, +0xc decay, +0x10 release (pitch / time units).
    pub a: f32,
    pub h: f32,
    pub d: f32,
    pub r: f32,
    /// +0x18 attack rate, +0x20 hold length (samples), +0x28 decay rate, +0x30
    /// sustain factor, +0x38 release factor.
    pub att_rate: f64,
    pub hold_len: f64,
    pub dec_rate: f64,
    pub sus: f64,
    pub rel: f64,
    /// +0x40 attack step, +0x48 decay step, +0x50 sustain level, +0x58 peak (f32),
    /// +0x60 peak, +0x68 value, +0x70 hold counter.
    pub att_inc: f64,
    pub dec_inc: f64,
    pub sus_lvl: f64,
    pub peak_f: f32,
    pub peak: f64,
    pub value: f64,
    pub hold_n: u32,
    /// +0x74 sample rate, +0x78 1/sr.
    pub sr: f32,
    pub inv: f64,
}

impl MemoEnv {
    pub fn new(a: f32, h: f32, d: f32, r: f32) -> MemoEnv {
        MemoEnv { a, h, d, r, ..Default::default() }
    }

    /// The release factor for release parameter `r`: `1 − E(rate)/sr`, or 0.
    #[inline]
    pub fn rel_for(&self, r: f32) -> f64 {
        let k = self.inv * e_rate(r) as f64;
        if k < 1.0 { 1.0 - k } else { 0.0 }
    }

    /// `setSampleRate`.
    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr;
        self.inv = 1.0 / sr as f64;
        self.att_rate = self.inv * e_rate(self.a) as f64;
        self.hold_len = ((e_time(self.h) + -0.001) * sr) as f64;
        self.dec_rate = self.inv * e_rate(self.d) as f64;
        self.rel = self.rel_for(self.r);
    }

    /// `setNoteOn(level)` (`level` in [0, 1]).
    pub fn note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        let st = self.state;
        let mut p = v;
        if st != 0 {
            let cur = self.value as f32;
            p = cur + v;
            let th = 1.0 - cur;
            if p >= th {
                let w = 1.0 - th;
                let w = if w < 1e-12 { 1e-12 } else { w };
                let x = p - th;
                let x = if x > w + w { w + w } else { x };
                p = x.mul_add((x * -0.25) / w + 1.0, th);
            }
        }
        self.peak_f = p;
        let pd = p as f64;
        self.peak = pd;
        self.att_inc = self.att_rate * pd;
        let s = self.sus * pd;
        self.dec_inc = (s - pd) * self.dec_rate;
        self.sus_lvl = s;
        let v0 = self.value;
        match st {
            0 => self.state = 1,
            4 => {
                if v0 <= pd {
                    self.state = 1;
                    if v0 >= pd {
                        self.state = 2;
                    }
                }
            }
            3 => {
                if v0 <= pd {
                    self.state = 1;
                    if v0 >= pd {
                        self.state = 2;
                    }
                } else if s >= v0 {
                    self.state = 4;
                }
            }
            2 => {
                let mut n = if self.sus == 1.0 { 4 } else { 3 };
                if v0 <= pd {
                    n = 1;
                }
                self.state = n;
                self.hold_n = 0;
                match n {
                    1 if v0 >= pd => self.state = 2,
                    3 if s >= v0 => self.state = 4,
                    _ => {}
                }
            }
            1 if v0 >= pd => self.state = 2,
            _ => {}
        }
    }

    /// One sample (`processSample`, inlined everywhere).
    #[inline]
    pub fn process(&mut self) -> f32 {
        let v = match self.state {
            0 => return 0.0,
            1 => {
                let v = self.att_inc + self.value;
                self.value = v;
                if v >= self.peak {
                    self.value = self.peak;
                    self.state = 2;
                    self.peak
                } else {
                    v
                }
            }
            2 => {
                self.hold_n = self.hold_n.wrapping_add(1);
                if self.hold_len <= self.hold_n as f64 {
                    self.state = 3;
                    self.hold_n = 0;
                }
                return self.peak_f;
            }
            3 => {
                let mut v = self.dec_inc + self.value;
                self.value = v;
                if self.sus_lvl >= v {
                    self.value = self.sus_lvl;
                    self.state = 4;
                    v = self.sus_lvl;
                }
                self.floor(v)
            }
            4 => {
                let v = self.rel * self.value;
                self.value = v;
                self.floor(v)
            }
            _ => self.value,
        };
        v as f32
    }

    #[inline]
    fn floor(&mut self, v: f64) -> f64 {
        if v < f64::from_bits(0x3f1a_36e2_e000_0000) {
            self.state = 0;
            self.value = 0.0;
            0.0
        } else {
            v
        }
    }
}
