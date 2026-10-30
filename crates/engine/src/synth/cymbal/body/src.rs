//! `NI::AB::SampleRateConverter`, generic type (non-integer ratios), quality 2, one
//! channel: `PinkSampler::setSampleRate` resamples the pink-noise recording with it once.
//! Copied from `crate::fx::cabinet` (two channels there).
//!
//! Integer ratios (88.2 kHz, 22.05 kHz) use Maschine's `init_intrat` converters
//! instead, which this does not cover.

const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}

const KERNEL: [[f32; 6]; 4] = [
    [c(0x3f80_0000), 0.0, c(0xbfef_bf40), c(0xbdab_07b6), c(0x3fc9_948e), c(0xbf1e_49a5)],
    [c(0x3ee7_235d), c(0x4046_ad79), c(0xc10d_895e), c(0x40f7_7d97), c(0xc034_3706), c(0x3ebe_7b5c)],
    [c(0x4110_2672), c(0xc1ad_d1f9), c(0x419d_38bb), c(0xc107_b3d9), c(0x3fe2_31fe), c(0xbe12_d342)],
    [c(0xc0bd_1d3c), c(0x4125_984e), c(0xc0dc_3032), c(0x400d_18b9), c(0xbeaf_eafa), c(0x3cab_bd9a)],
];
const K_POINTS: u32 = 8;
const K_ORDER: usize = 5;

/// `C(j, m)`.
fn binom(j: usize, m: usize) -> f32 {
    let (mut num, mut den) = (1u32, 1u32);
    for k in 0..(j - m) as u32 {
        num *= j as u32 - k;
        den *= k + 1;
    }
    (num / den) as f32
}

/// `NI::AB::SampleRateConverter` of type "generic" (any non-integer ratio), quality 2,
/// one channel (docs/fx-cabinet.md §4.1). Each output sample is an FIR over the last `taps` inputs
/// whose coefficients are polynomials in the output's fractional phase `x`: per tap, a
/// Taylor-shifted kernel segment, switching to the next segment once `x` passes the
/// tap's boundary. Downsampling stretches the kernel by `in/out` and scales it by
/// `out/in`.
pub(in super::super) struct Src {
    taps: usize,
    /// Per group of four taps: boundaries, then (lo, hi) coefficient vectors for
    /// `x^5 … x^0`; `groups·(2·5 + 3)` vectors of 4.
    table: Vec<[f32; 4]>,
    /// Inputs per output: `step` + `rem/out`.
    step: u32,
    rem: f32,
    out: f32,
    inv_out: f32,
    /// Input samples still to take before the next output, and the output's phase.
    pending: u32,
    phase: f32,
    /// History per channel, newest at `taps − 1` (padded to whole groups with zeros).
    hist: Vec<f32>,
}

impl Src {
    /// `init(fin, fout, 2, quality 2)` for a non-integer ratio (`init_generic`).
    fn new(fin: f32, fout: f32) -> Src {
        let r = fout / fin;
        let (mut step, mut rem) = (0, fin);
        if fout <= fin {
            let mut f = fin;
            loop {
                step += 1;
                f -= fout;
                if fout > f {
                    break;
                }
            }
            rem = f;
        }
        let taps = if r > 1.0 { K_POINTS as usize } else { ((K_POINTS as f64 * fin as f64) / fout as f64).ceil() as usize };
        let groups = taps.div_ceil(4);
        let (da, db, g) = if r > 1.0 { (2.0f64, 2.0f64, 1.0f32) } else { (2.0 * fout as f64, 2.0 * fin as f64, fout / fin) };
        let base = (db * K_POINTS as f64) * -0.5;
        let n = groups * 4;
        let mut bound = vec![0f32; n];
        let mut lo = vec![vec![0f32; n]; K_ORDER + 1];
        let mut hi = vec![vec![0f32; n]; K_ORDER + 1];
        let half = K_POINTS / 2;
        for i in 0..taps {
            let idx = taps - 1 - i;
            let di = i as f64;
            let t = di.mul_add(da, base);
            let a = if t >= 0.0 { t } else { -di.mul_add(da, base) };
            let q = a / db;
            let seg0 = q as u32;
            let rm = (-(seg0 as f64)).mul_add(db, a);
            let (down, d16) = if rm != 0.0 { (0, rm) } else { ((t < 0.0) as u32, db) };
            let dist = if t >= 0.0 { db - rm } else { d16 };
            let seg = seg0 - down;
            let dir: i32 = if t < 0.0 { -((seg != 0) as i32) } else { 1 };
            let s28 = if t >= 0.0 { g } else { -g };
            let flip: f32 = if da + t > 0.0 && t < 0.0 { -1.0 } else { 1.0 };
            let u0 = q as f32;
            let hseg = (dir + seg as i32) as u32;
            bound[idx] = (dist / da) as f32;
            let (s30, s31) = (flip * u0, s28 * flip);
            for m in 0..=K_ORDER {
                let mut v = 0f32;
                if seg < half {
                    for j in (m..=K_ORDER).rev() {
                        v = KERNEL[seg as usize][j].mul_add(binom(j, m), v * u0);
                    }
                    for _ in 0..m {
                        v *= s28;
                    }
                }
                lo[m][idx] = g * v;
                let mut v = 0f32;
                if hseg < half {
                    for j in (m..=K_ORDER).rev() {
                        v = KERNEL[hseg as usize][j].mul_add(binom(j, m), s30 * v);
                    }
                    for _ in 0..m {
                        v *= s31;
                    }
                }
                hi[m][idx] = g * v;
            }
        }
        let lanes = |v: &[f32], k: usize| -> [f32; 4] { std::array::from_fn(|j| v[4 * k + j]) };
        let mut table = Vec::with_capacity(groups * (2 * K_ORDER + 3));
        for k in 0..groups {
            table.push(lanes(&bound, k));
            for m in (0..=K_ORDER).rev() {
                table.push(lanes(&lo[m], k));
                table.push(lanes(&hi[m], k));
            }
        }
        Src { taps, table, step, rem, out: fout, inv_out: 1.0 / fout, pending: 1, phase: 0.0, hist: vec![0.0; n] }
    }

    /// How many outputs `n` more inputs make (`Cabinet::processAudio`): rounded up.
    fn outputs_for(&self, n: usize) -> usize {
        let num = (n as i64 - self.pending as i64 + 1) as f64 * self.out as f64 - self.phase as f64;
        let den = (self.step as f32).mul_add(self.out, self.rem) as f64;
        (num / den).ceil().max(0.0) as usize
    }

    /// Shift `k` inputs (from `pos`) into the history.
    fn take(&mut self, x: &[f32], pos: usize, k: usize) {
        let t = self.taps;
        let h = &mut self.hist;
        h.copy_within(k..t, 0);
        h[t - k..t].copy_from_slice(&x[pos..pos + k]);
    }

    /// `convert(input, output)` (`process_generic`): all of `input` is offered, and
    /// `output` is filled.
    fn convert(&mut self, input: &[f32], output: &mut [f32]) {
        let nin = input.len();
        let mut used = (self.pending as usize).min(nin);
        if used > 0 {
            self.take(input, 0, used);
        }
        self.pending -= used as u32;
        let groups = self.table.len() / (2 * K_ORDER + 3);
        let mut coef = vec![[0f32; 4]; groups];
        for o in 0..output.len() {
            let x = self.phase * self.inv_out;
            for (k, cf) in coef.iter_mut().enumerate() {
                let t = &self.table[k * (2 * K_ORDER + 3)..];
                let mask: [bool; 4] = std::array::from_fn(|j| x >= t[0][j]);
                let mut acc: [f32; 4] = std::array::from_fn(|j| if mask[j] { t[2][j] } else { t[1][j] });
                for m in 1..=K_ORDER {
                    let (l, h) = (t[1 + 2 * m], t[2 + 2 * m]);
                    for j in 0..4 {
                        acc[j] = acc[j] * x + if mask[j] { h[j] } else { l[j] };
                    }
                }
                *cf = acc;
            }
            let h = &self.hist;
            let mut s = [0f32; 4];
            for (k, cf) in coef.iter().enumerate() {
                for j in 0..4 {
                    let p = h[4 * k + j] * cf[j];
                    s[j] = if k == 0 { p } else { s[j] + p };
                }
            }
            output[o] = (s[0] + s[2]) + (s[1] + s[3]);
            let mut k = self.step as usize;
            self.phase += self.rem;
            if self.phase > self.out {
                self.phase -= self.out;
                k += 1;
            }
            if k > 0 {
                let total = used + k;
                if total > nin {
                    // Short of input: the rest is taken first in the next call.
                    self.pending = (total - nin) as u32;
                    let k = nin.saturating_sub(used).min(k);
                    if k > 0 {
                        self.take(input, used, k);
                    }
                } else {
                    self.take(input, used, k);
                }
                used = total;
            }
        }
    }
}

impl Src {
    /// `init(fin, fout, 1, 2)`, `resetAllChannels`, then `convert` of all of `x` into
    /// as many outputs as it makes (`PinkSampler::setSampleRate`).
    pub(in super::super) fn resample(fin: f32, fout: f32, x: &[f32]) -> Vec<f32> {
        let mut s = Src::new(fin, fout);
        let mut out = vec![0.0; s.outputs_for(x.len())];
        s.convert(x, &mut out);
        out
    }
}
