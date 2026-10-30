//! Maschine's side-chain filter (`FXCORE::SidechainFilterCore`; docs/fx-compressor.md §4,
//! docs/fx-limiter.md §5), shared by the Compressor, Limiter and Gate: it scales (and
//! optionally band-limits) the side-chain input in place before the effect's detector
//! sees it. Its parameters are the module's `side_chain_filter/*` fields.

use super::compressor::e;
use super::param;

/// An `f32` constant as Maschine stores it.
const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}
const K_DB: f32 = c(0x411f_73da); // 60 / (20·log10 2): dB → table index
const K_LN: f32 = c(0x42ad_1f97); // 60 / ln 2
const TWO_PI: f32 = c(0x40c9_0fdb);
const SQRT_HALF: f32 = c(0x3f35_04f3);

/// `(sin x, cos x)` as Maschine computes them (`__sincosf_stret` of Apple's libm).
fn sincosf(x: f32) -> (f32, f32) {
    #[cfg(target_os = "macos")]
    {
        #[repr(C)]
        struct SinCos {
            sin: f32,
            cos: f32,
        }
        unsafe extern "C" {
            fn __sincosf_stret(x: f32) -> SinCos;
        }
        // SAFETY: a pure libm function.
        let r = unsafe { __sincosf_stret(x) };
        (r.sin, r.cos)
    }
    #[cfg(not(target_os = "macos"))]
    {
        x.sin_cos()
    }
}

/// The normalized Center Freq/Width edge → Hz (`convertFilterCutoffNorm2Hz`, 20·1000ⁿ).
fn cutoff(n: f32) -> f32 {
    e(n.mul_add(c(0x42ef_2dc8), c(0x4177_ca04)).mul_add(5.0, c(0x44ac_bc36)))
}

/// Field names and defaults (normalized) of `DATA::SideChainFilter`: Gain, Center Freq,
/// Width, Filter.
const PARAMS: [(&str, f32); 4] = [
    ("side_chain_filter/gain", 1.0),
    ("side_chain_filter/center_freq", 0.5),
    ("side_chain_filter/width", 0.5),
    ("side_chain_filter/filter", 0.0),
];

pub struct SidechainFilter {
    inv_sr: f32,
    /// Samples since the last control tick (a tick every 128).
    counter: i32,
    center: f32,
    width: f32,
    on: bool,
    gain: f32,
    /// The band's edges, normalized.
    hi: f32,
    lo: f32,
    /// Lanes: low-pass at `hi` (L, R), high-pass at `lo` (L, R).
    a1: [f32; 4],
    a2: [f32; 4],
    // Ramped lanes: the input gain G (L, R) and the gain between the two filters (L, R,
    // always 1). §3.2 of docs/fx-compressor.md.
    cur: [f32; 4],
    inc: [f32; 4],
    tgt: [f32; 4],
    dirty: bool,
    countdown: i32,
    ticks: i32,
    inv: f32,
    // Biquad state per lane: the last two inputs and outputs.
    x1: [f32; 4],
    x2: [f32; 4],
    y1: [f32; 4],
    y2: [f32; 4],
    values: Option<[f32; 4]>,
}

impl SidechainFilter {
    /// The core as constructed, then `setSampleRate(sr)`.
    pub fn new(sr: f64) -> SidechainFilter {
        let mut f = SidechainFilter {
            inv_sr: 1.0,
            counter: 128,
            center: 0.0,
            width: 0.0,
            on: false,
            gain: 0.0,
            hi: 1.0,
            lo: 0.0,
            a1: [0.0; 4],
            a2: [0.0; 4],
            cur: [0.0, 0.0, 1.0, 1.0],
            inc: [0.0; 4],
            tgt: [0.0, 0.0, 1.0, 1.0],
            dirty: false,
            countdown: 0,
            ticks: 1,
            inv: 0.0,
            x1: [0.0; 4],
            x2: [0.0; 4],
            y1: [0.0; 4],
            y2: [0.0; 4],
            values: None,
        };
        f.set_sample_rate(sr as f32);
        f
    }

    /// `setSampleRate`.
    fn set_sample_rate(&mut self, sr: f32) {
        if sr == 0.0 {
            return;
        }
        self.inv_sr = 1.0 / sr;
        let n = ((sr * c(0x3ca3_d70a)) * 0.007_812_5 + 0.5) as i32;
        self.ticks = n.max(1);
        self.inv = 1.0 / (self.ticks << 7) as f32;
        self.coeffs();
    }

    /// `calcFilterCoeffs`: the two Butterworth sections and the input gain's target.
    fn coeffs(&mut self) {
        let (wh, wl) = (cutoff(self.hi) * TWO_PI * self.inv_sr, cutoff(self.lo) * TWO_PI * self.inv_sr);
        let (sh, ch) = sincosf(wh);
        let (sl, cl) = sincosf(wl);
        let (ah, al) = (sh * SQRT_HALF, sl * SQRT_HALF);
        let (nh, nl) = (1.0 / (ah + 1.0), 1.0 / (al + 1.0));
        let a1h = (ch + ch) * nh;
        let a1l = (cl + cl) * nl;
        let a2h = nh * -(1.0 - ah);
        let a2l = nl * -(1.0 - al);
        self.a1 = [a1h, a1h, a1l, a1l];
        self.a2 = [a2h, a2h, a2l, a2l];
        let b0lp = (1.0 - ch) * 0.5 * nh;
        let b0hp = (cl + 1.0) * 0.5 * nl;
        let g = if self.on {
            let k = e((self.width * -20.0 * SQRT_HALF).mul_add(K_LN, 1200.0)) + 1.0;
            k * (b0hp * (self.gain * b0lp))
        } else {
            self.gain
        };
        self.tgt[0] = g;
        self.tgt[1] = g;
        self.dirty = true;
    }

    /// The band edges `clamp(center ± width/2, 0, 1)` (`setParamCenterFreq`/`Width`).
    fn edges(&mut self) {
        let clamp = |x: f32| {
            let x = if x < 0.0 { 0.0 } else { x };
            if x > 1.0 { 1.0 } else { x }
        };
        self.hi = clamp(self.width.mul_add(0.5, self.center));
        self.lo = clamp(self.width.mul_add(-0.5, self.center));
    }

    fn apply(&mut self, k: usize, v: f32) {
        match k {
            0 => {
                // `setParamGain`: −20..+20 dB.
                self.gain = e(v.mul_add(40.0, -20.0).mul_add(K_DB, 1200.0));
            }
            1 => {
                self.center = v;
                self.edges();
            }
            2 => {
                self.width = v;
                self.edges();
            }
            _ => self.on = v > 0.5,
        }
        self.coeffs();
    }

    /// `resume`: the gains jump to their targets, the filters clear.
    fn resume(&mut self) {
        self.counter = 128;
        self.x1 = [0.0; 4];
        self.x2 = [0.0; 4];
        self.y1 = [0.0; 4];
        self.y2 = [0.0; 4];
        self.cur = self.tgt;
        self.dirty = true;
    }

    /// Take the module's `side_chain_filter/*` parameters. The first settings apply at
    /// once (they reach the core before the processor resumes); later changes ramp.
    pub fn set(&mut self, params: &[(String, f64)]) {
        let new = PARAMS.map(|(name, def)| param(params, name, def as f64) as f32);
        match self.values {
            None => {
                for (k, v) in new.iter().enumerate() {
                    self.apply(k, *v);
                }
                self.resume();
            }
            Some(old) => {
                for k in 0..4 {
                    if new[k].to_bits() != old[k].to_bits() {
                        self.apply(k, new[k]);
                    }
                }
            }
        }
        self.values = Some(new);
    }

    /// A control tick of the gain ramps.
    fn tick(&mut self) {
        if self.dirty {
            self.dirty = false;
            let mut any = false;
            for k in 0..4 {
                self.inc[k] = (self.tgt[k] - self.cur[k]) * self.inv;
                if ((self.inc[k] * self.inc[k]) as f64) < 1e-15 {
                    self.inc[k] = 0.0;
                    self.cur[k] = self.tgt[k];
                } else {
                    any = true;
                }
            }
            if any {
                self.countdown = self.ticks;
            }
        } else {
            self.countdown = self.countdown.wrapping_sub(1);
            if self.countdown == 0 {
                self.cur = self.tgt;
                self.inc = [0.0; 4];
            }
        }
    }

    /// `processAudio`: filter `l`/`r` (the same length) in place.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        const NUM: [f32; 4] = [2.0, 2.0, -2.0, -2.0];
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            for k in 0..4 {
                self.cur[k] += self.inc[k];
            }
            if self.counter < 127 {
                self.counter += 1;
            } else {
                self.tick();
                self.counter = 0;
            }
            if self.on {
                // Low-pass on (L, R)·G, then high-pass on the previous low-pass output.
                let x = [*xl * self.cur[0], *xr * self.cur[1], self.y1[0] * self.cur[2], self.y1[1] * self.cur[3]];
                let mut y = [0f32; 4];
                for k in 0..4 {
                    let t = self.x2[k] + (self.x1[k] * NUM[k] + x[k]);
                    y[k] = self.a2[k] * self.y2[k] + (self.a1[k] * self.y1[k] + t);
                }
                self.x2 = self.x1;
                self.x1 = x;
                self.y2 = self.y1;
                self.y1 = y;
                (*xl, *xr) = (y[2], y[3]);
            } else {
                *xl *= self.cur[0];
                *xr *= self.cur[1];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(gain: f64, center: f64, width: f64, filter: f64) -> Vec<(String, f64)> {
        [("gain", gain), ("center_freq", center), ("width", width), ("filter", filter)]
            .iter()
            .map(|(k, v)| (format!("side_chain_filter/{k}"), *v))
            .collect()
    }

    fn sine(hz: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (i as f32 * hz / 48_000.0 * std::f32::consts::TAU).sin()).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        let t = &x[x.len() / 2..];
        (t.iter().map(|v| v * v).sum::<f32>() / t.len() as f32).sqrt()
    }

    fn run(p: &[(String, f64)], x: &[f32]) -> Vec<f32> {
        let mut f = SidechainFilter::new(48_000.0);
        f.set(p);
        let (mut l, mut r) = (x.to_vec(), x.to_vec());
        f.process(&mut l, &mut r);
        assert_eq!(l, r);
        l
    }

    #[test]
    fn gain_spans_40_db() {
        let x = sine(1000.0, 4800);
        let db = |g| 20.0 * (rms(&run(&params(g, 0.5, 0.5, 0.0), &x)) / rms(&x)).log10();
        assert!((db(1.0) - 20.0).abs() < 0.05, "{}", db(1.0));
        assert!((db(0.5)).abs() < 0.05, "{}", db(0.5));
        assert!((db(0.0) + 20.0).abs() < 0.05, "{}", db(0.0));
    }

    #[test]
    fn filter_passes_the_band() {
        // Centre 632 Hz (0.5), narrow: 632 Hz passes, 50 Hz and 10 kHz don't.
        let p = params(0.5, 0.5, 0.2, 1.0);
        let level = |hz| rms(&run(&p, &sine(hz, 9600)));
        let (lo, mid, hi) = (level(50.0), level(632.0), level(10_000.0));
        assert!(mid > 4.0 * lo && mid > 4.0 * hi, "{lo} {mid} {hi}");
    }

    #[test]
    fn gain_changes_ramp() {
        let mut f = SidechainFilter::new(48_000.0);
        f.set(&params(0.5, 0.5, 0.5, 0.0));
        f.set(&params(1.0, 0.5, 0.5, 0.0));
        let (mut l, mut r) = (vec![1.0f32; 2400], vec![1.0f32; 2400]);
        f.process(&mut l, &mut r);
        assert!((l[0] - 1.0).abs() < 1e-3 && l[600] > 1.5 && l[600] < 9.0, "{} {}", l[0], l[600]);
        assert!((l[2000] - 10.0).abs() < 1e-3, "{}", l[2000]);
    }
}
