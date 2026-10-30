//! Maschine's EQ (docs/fx-eq.md): `RT::EQProcessor` → `FXCORE::GBXEQCore`. Two 4-lane
//! biquad stages per sample (low/high shelf, then the two peaks), with the Low-Mid → High
//! hand-off one sample late, so the EQ has 1 sample of latency. All `f32`; `mul_add` only
//! where the original fuses (`fmadd`), so the output can match bit for bit.

use super::exp_e;

use super::{Effect, param};

const HALF_PI: f32 = f32::from_bits(0x3fc9_0fdb);
const TWO_PI: f32 = f32::from_bits(0x40c9_0fdb);
/// 3/log10(2): dB·K + 1200 indexes the exp table as 10^(dB/20).
const K_DB: f32 = f32::from_bits(0x411f_73da);
/// 60/ln 2: y·K + 1200 indexes the exp table as e^y.
const K_E: f32 = f32::from_bits(0x42ad_1f97);
const LN2_HALF: f32 = f32::from_bits(0x3eb1_7218);
const W_MIN: f32 = f32::from_bits(0x3727_c5ac); // 1e-5
const WIDTH_SCALE: f32 = f32::from_bits(0x4079_999a); // 3.9
const WIDTH_MIN: f32 = f32::from_bits(0x3dcc_cccd); // 0.1
const BLOCK: i32 = 128;

/// `FloatExponentialConverter(min, max)` (base 10).
struct ExpConv {
    min: f32,
    max: f32,
    lmin: f32,
    range: f32,
}

impl ExpConv {
    fn new(min: f32, max: f32) -> ExpConv {
        let lmin = (min as f64).log10() as f32;
        ExpConv { min, max, lmin, range: (max as f64).log10() as f32 - lmin }
    }

    fn to(&self, v: f32) -> f32 {
        let f = 10f32.powf(v.mul_add(self.range, self.lmin));
        let f = if f < self.min { self.min } else { f };
        if f > self.max { self.max } else { f }
    }

    fn from(&self, h: f32) -> f32 {
        let v = if h > 0.0 { (h.log10() / 10f32.log10() - self.lmin) / self.range } else { h };
        v.clamp(0.0, 1.0)
    }
}

/// Polynomial sin/cos of `w` (§2.4). Returns (sin, cos, w > π/2, π/2 − (w − π/2)).
fn sin_cos(w: f32) -> (f32, f32, bool, f32) {
    let gt = w > HALF_PI;
    let red = HALF_PI - (w + -HALF_PI);
    let x = if gt { red } else { w };
    let x2 = x * x;
    let p = (x2 * f32::from_bits(0xbdaa_aaab)).mul_add(x2.mul_add(f32::from_bits(0xbd08_8889), 1.0), 1.0);
    let q = (x2 * f32::from_bits(0xbd4c_cccd)).mul_add(x2.mul_add(f32::from_bits(0xbcc3_0c30), 1.0), 1.0);
    let h = x2 * -0.5;
    let sin = x * (x2 * f32::from_bits(0xbe2a_aaab)).mul_add(q, 1.0);
    let c = h.mul_add(p, 1.0);
    (sin, if gt { -c } else { c }, gt, red)
}

/// Biquad coefficients as stored per lane: `[inv_a0, a1/a0, a2/a0, b0, b1, b2]`.
type Coefs = [f32; 6];

/// RBJ shelf, S = 0.5 (§2.4), decoded lane by lane from `processAudio`.
fn shelf(w: f32, g: f32, high: bool) -> Coefs {
    let a = g * g;
    let ia = 1.0 / a;
    let (sin, cos, _, _) = sin_cos(w);
    let kt = g * ((((a + 2.0) + ia).sqrt() * (sin * 0.5)) * -2.0);
    let (ap, am) = (a + 1.0, a + -1.0);
    let (a0, a2, b0, b2, a1, b1);
    if high {
        let lo = ap - am * cos;
        let hi = ap + am * cos;
        a0 = lo - kt;
        a2 = kt + lo;
        b0 = a * (hi - kt);
        b2 = a * (kt + hi);
        a1 = 2.0 * (am - ap * cos);
        b1 = (a * -2.0) * (am + ap * cos);
    } else {
        let hi = ap + am * cos;
        let lo = ap - am * cos;
        a0 = hi - kt;
        a2 = kt + hi;
        b0 = a * (lo - kt);
        b2 = a * (lo + kt);
        b1 = (a + a) * (am - ap * cos);
        a1 = -2.0 * (am + ap * cos);
    }
    let inv = 1.0 / a0;
    [inv, a1 * inv, inv * a2, b0, b1, b2]
}

/// RBJ peak with bandwidth in octaves and α ≤ bw (§2.4).
fn peak(w: f32, g: f32, bw: f32) -> Coefs {
    let (sin, cos, gt, red) = sin_cos(w);
    let w2 = if gt { HALF_PI - (red + -HALF_PI) } else { w };
    let y = ((bw * w2) * LN2_HALF) / sin;
    let ep = exp_e(y.mul_add(K_E, 1200.0));
    let em = exp_e(y.mul_add(-K_E, 1200.0));
    let al = (sin * 0.5) * (ep - em);
    let al = if al < bw { al } else { bw };
    let a1 = cos * -2.0;
    let inv = g / (g + al);
    [inv, a1 * inv, inv * (-(al / g) + 1.0), g * al + 1.0, a1, -(g * al) + 1.0]
}

// Smoother lanes (`SmoothingToolbox`, §2.5).
const W_LO: usize = 0;
const W_M1: usize = 1;
const G_LO: usize = 2;
const G_M1: usize = 3;
const W_M2: usize = 4;
const W_HI: usize = 5;
const G_M2: usize = 6;
const G_HI: usize = 7;
const BW_M1: usize = 8;
const BW_M2: usize = 9;
const G_OUT: usize = 10;

/// Parameter fields in RT-index order.
const FIELDS: [&str; 11] = [
    "low_freq",
    "low_gain",
    "low_mid_freq",
    "low_mid_gain",
    "high_mid_freq",
    "high_mid_gain",
    "high_freq",
    "high_gain",
    "low_mid_width",
    "high_mid_width",
    "output_gain",
];

pub struct Eq {
    sr: f32,
    inv_sr: f32,
    lf: ExpConv,
    mf: ExpConv,
    hf: ExpConv,
    /// Stored (normalized) values, RT order.
    vals: [f32; 11],
    /// No audio yet: new settings apply as Maschine's `resume` does (no ramp).
    fresh: bool,
    first: bool,
    dirty: bool,
    cur: [f32; 12],
    inc: [f32; 12],
    tgt: [f32; 12],
    steps: i32,
    remaining: i32,
    inv_len: f32,
    counter: i32,
    last_sh: [f32; 4],
    last_pk: [f32; 6],
    lo: Coefs,
    hi: Coefs,
    m1: Coefs,
    m2: Coefs,
    st1: [[f32; 4]; 6],
    st2: [[f32; 4]; 6],
    s1: [[f32; 4]; 2],
    s2: [[f32; 4]; 2],
    /// Low-Mid output of the previous frame (feeds the High lanes).
    d: [f32; 2],
}

impl Eq {
    pub fn new(sr: f64) -> Eq {
        let (lf, mf, hf) = (ExpConv::new(20.0, 8000.0), ExpConv::new(40.0, 16000.0), ExpConv::new(50.0, 20000.0));
        let vals = [lf.from(248.0), 0.0, mf.from(630.0), 0.0, mf.from(1970.0), 0.0, hf.from(2610.0), 0.0, 0.5, 0.5, 0.0];
        let sr = sr as f32;
        let mut eq = Eq {
            sr,
            inv_sr: 1.0 / sr,
            lf,
            mf,
            hf,
            vals,
            fresh: true,
            first: true,
            dirty: true,
            cur: [0.0; 12],
            inc: [0.0; 12],
            tgt: [0.0; 12],
            steps: 1,
            remaining: 0,
            inv_len: 1.0,
            counter: 0,
            last_sh: [0.0; 4],
            last_pk: [0.0; 6],
            lo: [0.0; 6],
            hi: [0.0; 6],
            m1: [0.0; 6],
            m2: [0.0; 6],
            st1: [[0.0; 4]; 6],
            st2: [[0.0; 4]; 6],
            s1: [[0.0; 4]; 2],
            s2: [[0.0; 4]; 2],
            d: [0.0; 2],
        };
        eq.resume();
        eq
    }

    /// The smoothing target for RT parameter `idx` (processEventIn*, §1.3).
    fn target(&mut self, idx: usize) {
        let v = self.vals[idx];
        let inv_sr = self.inv_sr;
        let w = |f: f32| (inv_sr * (f * TWO_PI)).max(W_MIN);
        let gain = |db: f32, k: f32| exp_e((db * k).mul_add(K_DB, 1200.0));
        match idx {
            0 => self.tgt[W_LO] = w(self.lf.to(v)),
            1 => self.tgt[G_LO] = gain(v * 20.0, 0.25),
            // (1/sr · 2π) · f in this one (endInit and processEventInM1F).
            2 => self.tgt[W_M1] = ((inv_sr * TWO_PI) * self.mf.to(v)).max(W_MIN),
            3 => self.tgt[G_M1] = gain(v * 20.0, 0.5),
            4 => self.tgt[W_M2] = w(self.mf.to(v)),
            5 => self.tgt[G_M2] = gain(v * 20.0, 0.5),
            6 => self.tgt[W_HI] = w(self.hf.to(v)),
            7 => self.tgt[G_HI] = gain(v * 20.0, 0.25),
            8 => self.tgt[BW_M1] = v.mul_add(WIDTH_SCALE, WIDTH_MIN),
            9 => self.tgt[BW_M2] = v.mul_add(WIDTH_SCALE, WIDTH_MIN),
            _ => self.tgt[G_OUT] = gain(v * 20.0, 1.0),
        }
    }

    /// `EQProcessor::resume` = `beginInit` + `endInit` (§2.6): clear all state, targets
    /// from the stored values, no ramp.
    fn resume(&mut self) {
        let block = BLOCK as f32;
        self.inv_sr = 1.0 / self.sr;
        self.steps = (((self.sr * f32::from_bits(0x3ca3_d70a)) / block + 0.5) as i32).max(1);
        self.inv_len = 1.0 / (self.steps * BLOCK) as f32;
        self.inc = [0.0; 12];
        self.tgt = [0.0; 12];
        for i in 0..11 {
            self.target(i);
        }
        self.cur = self.tgt;
        self.remaining = 0;
        self.counter = 0;
        self.first = true;
        self.dirty = true;
        self.s1 = [[0.0; 4]; 2];
        self.s2 = [[0.0; 4]; 2];
        self.d = [0.0; 2];
    }

    /// `SmoothingToolbox::startSmooth`.
    fn start_smooth(&mut self) {
        let mut sum = 0f32;
        for i in 0..12 {
            let inc = (self.tgt[i] - self.cur[i]) * self.inv_len;
            let sq = inc * inc;
            if (sq as f64) < f64::from_bits(0x3cd2_03af_9ee7_5616) {
                self.inc[i] = 0.0;
                self.cur[i] = self.tgt[i];
            } else {
                self.inc[i] = inc;
                sum += sq;
            }
        }
        if sum > 0.0 {
            self.remaining = self.steps;
        }
    }

    /// `GBXEQCore::processAudio`: one stereo frame.
    #[inline]
    fn frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        for i in 0..12 {
            self.cur[i] += self.inc[i];
        }
        self.counter += 1;
        let first = self.first;
        if first || self.counter >= BLOCK {
            self.counter = 0;
            if self.dirty {
                self.dirty = false;
                self.start_smooth();
            } else {
                self.remaining = self.remaining.wrapping_sub(1);
                if self.remaining == 0 {
                    self.cur = self.tgt;
                    self.inc = [0.0; 12];
                }
            }
        }
        let c = self.cur;
        // Stage 1: shelves, rebuilt when either changed.
        let mut sh = first;
        if c[W_LO] != self.last_sh[0] || c[G_LO] != self.last_sh[1] || first {
            self.last_sh[..2].copy_from_slice(&[c[W_LO], c[G_LO]]);
            self.lo = shelf(c[W_LO], c[G_LO], false);
            sh = true;
        }
        if c[W_HI] != self.last_sh[2] || c[G_HI] != self.last_sh[3] || first {
            self.last_sh[2..].copy_from_slice(&[c[W_HI], c[G_HI]]);
            self.hi = shelf(c[W_HI], c[G_HI], true);
            sh = true;
        }
        if sh {
            for k in 0..6 {
                self.st1[k] = [self.lo[k], self.lo[k], self.hi[k], self.hi[k]];
            }
        }
        // Stage 2: peaks.
        let mut pk = first;
        if c[W_M1] != self.last_pk[0] || c[G_M1] != self.last_pk[1] || c[BW_M1] != self.last_pk[2] || first {
            self.last_pk[..3].copy_from_slice(&[c[W_M1], c[G_M1], c[BW_M1]]);
            self.m1 = peak(c[W_M1], c[G_M1], c[BW_M1]);
            pk = true;
        }
        if c[W_M2] != self.last_pk[3] || c[G_M2] != self.last_pk[4] || c[BW_M2] != self.last_pk[5] || first {
            self.last_pk[3..].copy_from_slice(&[c[W_M2], c[G_M2], c[BW_M2]]);
            self.m2 = peak(c[W_M2], c[G_M2], c[BW_M2]);
            pk = true;
        }
        if pk {
            for k in 0..6 {
                self.st2[k] = [self.m1[k], self.m1[k], self.m2[k], self.m2[k]];
            }
        }
        self.first = false;

        let g = c[G_OUT];
        let x = [l * g, r * g, self.d[0], self.d[1]];
        let y = biquad4(&self.st1, &mut self.s1, x);
        let z = biquad4(&self.st2, &mut self.s2, y);
        self.d = [z[0], z[1]];
        (z[2], z[3])
    }
}

/// Four TDF-II lanes with unnormalized b and the state scaled by a0 (§2.3).
#[inline]
fn biquad4(c: &[[f32; 4]; 6], s: &mut [[f32; 4]; 2], x: [f32; 4]) -> [f32; 4] {
    let mut y = [0f32; 4];
    for i in 0..4 {
        let v = s[0][i] + c[3][i] * x[i];
        s[0][i] = s[1][i] + (c[4][i] * x[i] - c[1][i] * v);
        s[1][i] = c[5][i] * x[i] - c[2][i] * v;
        y[i] = v * c[0][i];
    }
    y
}

impl Effect for Eq {
    fn set(&mut self, params: &[(String, f64)]) {
        let mut changed = false;
        for (i, f) in FIELDS.iter().enumerate() {
            let v = param(params, f, self.vals[i] as f64) as f32;
            if v != self.vals[i] {
                self.vals[i] = v;
                changed = true;
                if !self.fresh {
                    self.target(i);
                }
            }
        }
        if self.fresh {
            // Maschine loads the values, then resumes the processor before any audio.
            self.resume();
        } else if changed {
            self.dirty = true;
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.fresh = false;
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            (*a, *b) = self.frame(*a, *b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(fx: &mut Eq, x: &[f32]) -> Vec<f32> {
        let (mut l, mut r) = (x.to_vec(), x.to_vec());
        for (a, b) in l.chunks_mut(300).zip(r.chunks_mut(300)) {
            fx.process(a, b);
        }
        l
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn exp_table_is_ten_to_the_db() {
        assert!((exp_e(1200.0) - 1.0).abs() < 1e-6);
        assert!((exp_e(6.0f32.mul_add(K_DB, 1200.0)) - 10f32.powf(0.3)).abs() < 1e-4);
    }

    #[test]
    fn flat_is_a_one_sample_delay() {
        let mut fx = Eq::new(48_000.0);
        fx.set(&[]);
        let x: Vec<f32> = (0..4000).map(|i| ((i * 7919) % 1000) as f32 / 1000.0 - 0.5).collect();
        let y = run(&mut fx, &x);
        assert_eq!(y[0], 0.0);
        for i in 1..x.len() {
            assert!((y[i] - x[i - 1]).abs() < 1e-5, "{i}: {} vs {}", y[i], x[i - 1]);
        }
    }

    #[test]
    fn a_boost_raises_its_band() {
        let sr = 48_000.0;
        let f = 1000.0f32;
        let x: Vec<f32> = (0..48_000).map(|i| (i as f32 * TWO_PI * f / sr as f32).sin() * 0.25).collect();
        let level = |p: &[(String, f64)]| {
            let mut fx = Eq::new(sr);
            fx.set(p);
            rms(&run(&mut fx, &x)[4800..])
        };
        let flat = level(&[]);
        let mf = ExpConv::new(40.0, 16000.0).from(f) as f64;
        let boost = level(&[("low_mid_freq".into(), mf), ("low_mid_gain".into(), 0.5)]);
        let db = 20.0 * (boost / flat).log10();
        assert!((db - 10.0).abs() < 0.1, "+10 dB peak gave {db} dB");
        let cut = level(&[("output_gain".into(), -0.3)]);
        let db = 20.0 * (cut / flat).log10();
        assert!((db + 6.0).abs() < 0.01, "−6 dB output gave {db} dB");
    }

    #[test]
    fn changes_ramp_without_reset() {
        let mut fx = Eq::new(44_100.0);
        fx.set(&[]);
        let x = vec![0.5f32; 10_000];
        let mut y = run(&mut fx, &x[..2000]);
        fx.set(&[("output_gain".into(), -1.0)]);
        y.extend(run(&mut fx, &x[2000..]));
        // A smooth ramp from 1 to 0.1 (−20 dB) over 7 blocks, from the next block on.
        assert!((y[1999] - 0.5).abs() < 1e-5);
        assert!((y[9999] - 0.05).abs() < 1e-4, "{}", y[9999]);
        let jump = y[1..].windows(2).map(|w| (w[1] - w[0]).abs()).fold(0f32, f32::max);
        assert!(jump < 1e-3, "jump {jump}");
    }
}
