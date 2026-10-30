//! Maschine's Compressor (docs/fx-compressor.md): Classic (`FXCORE::CompressorCore`, §5.1)
//! and Feedback (`FXCORE::FBCompressorCore`, §5.2), chosen by Mode. Both cores always take
//! every parameter; only the chosen one runs, and the other keeps its state (§2). With a
//! side-chain source, the side-chain filter (§4) shapes it and the detectors see it.

/// The table lookup `E(x) ≈ 2^((x − 1200)/60)` (§3.1).
pub(super) use super::exp_e as e;
use super::sidechain::SidechainFilter;
use super::{Effect, param};

/// An `f32` constant as Maschine stores it.
const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}
const K_DB: f32 = c(0x411f_73da); // 60 / (20·log10 2): dB → table index
const K_LN: f32 = c(0x42ad_1f97); // 60 / ln 2: natural log → table index
const LN2: f32 = c(0x3f31_7215);
const TINY: f32 = c(0x0da2_4260); // 1e-30
const W_MIN: f32 = c(0x3a83_126f); // 0.001
const D: f32 = c(0x2193_92ef); // 1e-18, the Feedback core's denormal offset

/// `1/x` as NEON `frecpe` and two `frecps` Newton steps.
fn recip(x: f32) -> f32 {
    #[cfg(target_arch = "aarch64")]
    {
        use std::arch::aarch64::*;
        // SAFETY: NEON is always present on aarch64.
        unsafe {
            let v = vdup_n_f32(x);
            let r = vrecpe_f32(v);
            let r = vmul_f32(r, vrecps_f32(v, r));
            let r = vmul_f32(vrecps_f32(v, r), r);
            vget_lane_f32::<0>(r)
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let r = frecpe(x);
        let r = r * (-x).mul_add(r, 2.0);
        (-x).mul_add(r, 2.0) * r
    }
}

/// ARM's `FRECPE` (an 8-bit reciprocal estimate) for normal, finite `x`.
#[cfg(any(not(target_arch = "aarch64"), test))]
fn frecpe(x: f32) -> f32 {
    let b = x.to_bits();
    let (sign, exp, frac) = (b & 0x8000_0000, ((b >> 23) & 0xff) as i32, b & 0x7f_ffff);
    if exp == 0 || exp == 0xff {
        return 1.0 / x;
    }
    let a = (0x100 | (frac >> 15)) * 2 + 1;
    let est = ((1u32 << 19) / a).div_ceil(2);
    let mut f = (est & 0xff) << 15;
    let mut re = 253 - exp;
    if re == 0 {
        f = 0x40_0000 | (f >> 1);
    } else if re == -1 {
        f = 0x20_0000 | (f >> 2);
        re = 0;
    } else if re < -1 {
        return f32::from_bits(sign);
    }
    f32::from_bits(sign | ((re as u32) << 23) | f)
}

/// The parameter ramps of §3.2 over `N` lanes: linear, `ticks` control ticks of 128
/// samples long.
struct Ramp<const N: usize> {
    cur: [f32; N],
    inc: [f32; N],
    tgt: [f32; N],
    dirty: bool,
    countdown: i32,
    ticks: i32,
    inv: f32,
}

impl<const N: usize> Ramp<N> {
    fn new(sr: f32) -> Self {
        let ticks = (((sr * c(0x3ca3_d70a)) / 128.0 + 0.5) as i32).max(1);
        Ramp { cur: [0.0; N], inc: [0.0; N], tgt: [0.0; N], dirty: false, countdown: 0, ticks, inv: 1.0 / (ticks * 128) as f32 }
    }

    fn step(&mut self) {
        for k in 0..N {
            self.cur[k] += self.inc[k];
        }
    }

    /// A control tick.
    fn tick(&mut self) {
        if self.dirty {
            self.dirty = false;
            let mut any = false;
            for k in 0..N {
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
                self.inc = [0.0; N];
            }
        }
    }
}

// ---- Classic (§5.1) ------------------------------------------------------------------

// Ramp lanes.
const C_T: usize = 0;
const C_TA: usize = 1;
const C_ITA: usize = 2;
const C_ITR: usize = 3;
const C_S: usize = 4;
const C_K2: usize = 5;
const C_G: usize = 6;

struct Classic {
    inv_sr: f32,
    ramp: Ramp<8>,
    tick: i32,
    latch: i32,
    /// T, Ta, 1/Ta, 1/Tr, K2, S, Gdb as last latched.
    lat: [f32; 7],
    prev_in: [f32; 2],
    dly: [f32; 2],
    env: f32,
    lim: f32,
    winv: f32,
    wraw: f32,
    w: f32,
    /// The reduction the last sample took (dB, before the makeup), for the GR meter.
    reduction: f32,
}

impl Classic {
    fn new(sr: f32) -> Classic {
        Classic {
            inv_sr: 1.0 / sr,
            ramp: Ramp::new(sr),
            tick: 0,
            latch: 0,
            lat: [0.0; 7],
            prev_in: [0.0; 2],
            dly: [0.0; 2],
            env: 0.0,
            lim: 0.0,
            winv: 0.0,
            wraw: 0.0,
            w: 0.0,
            reduction: 0.0,
        }
    }

    fn target(&mut self, lane: usize, v: f32) {
        self.ramp.tgt[lane] = v;
        self.ramp.dirty = true;
    }

    fn set_threshold(&mut self, v: f32) {
        self.target(C_T, v * -60.0);
    }

    fn set_amount(&mut self, v: f32) {
        self.target(C_S, (2.0 - v) * v);
    }

    fn set_knee(&mut self, v: f32) {
        self.target(C_K2, v * 0.5);
    }

    fn set_attack(&mut self, v: f32) {
        let ta = v.powf(3.0) * 8.0;
        let ta = if ta < c(0x2edb_e6ff) { c(0x2edb_e6ff) } else { ta };
        self.target(C_TA, ta);
        self.target(C_ITA, 1.0 / ta);
    }

    fn set_release(&mut self, v: f32) {
        let tr = c(0x3f8f_9e4d).powf(v.mul_add(16.0, 32.0));
        let tr = if tr < 40.0 { 40.0 } else { tr };
        let tr = if tr > 251.0 { 251.0 } else { tr };
        self.target(C_ITR, 1.0 / tr);
    }

    fn set_gain(&mut self, v: f32) {
        self.target(C_G, (v + -1.0) * 36.0);
    }

    /// `CompressorCore::resume`: parameters apply at once, state clears.
    fn resume(&mut self) {
        self.ramp.cur = self.ramp.tgt;
        self.ramp.inc = [0.0; 8];
        self.ramp.dirty = true;
        self.ramp.countdown = 0;
        // The first sample runs a control tick and a latch.
        self.tick = 129;
        self.latch = 5;
        self.prev_in = [0.0; 2];
        self.dly = [0.0; 2];
        self.env = 0.0;
        self.lim = 0.0;
        self.winv = 0.0;
        self.wraw = 0.0;
        self.w = 0.0;
    }

    /// One sample (`CompressorCore::processAudio`) of input `x` and detector input `d`
    /// (the side-chain, or `x`); the output is two samples late.
    fn process(&mut self, (xl, xr): (f32, f32), (sl, sr): (f32, f32)) -> (f32, f32) {
        self.ramp.step();
        self.tick += 1;
        if self.tick >= 128 {
            self.tick = 0;
            self.ramp.tick();
        }
        let old = self.latch;
        self.latch += 1;
        if old >= 4 {
            let r = &self.ramp.cur;
            self.lat = [r[C_T], r[C_TA], r[C_ITA], r[C_ITR], r[C_K2], r[C_S], r[C_G]];
            self.latch = 0;
        }
        let [t, ta, ita, itr, k2, s, gdb] = self.lat;

        // Detector: the stereo-linked peak, in dB over the threshold.
        let (al, ar) = (sl.abs(), sr.abs());
        let p = if al < ar { ar } else { al };
        let over = if p > e(t.mul_add(K_DB, 1200.0)) {
            let b = p.to_bits();
            let m = f32::from_bits((b & 0x807f_ffff) | 0x3f80_0000) + -1.0;
            let ex = ((b >> 23) & 0xff) as i32 - 127;
            let log2 = m * (m * c(0xbeb1_4749) + c(0x3fac_51ec)) + ex as f32;
            log2 * c(0x40c0_a82f) - t
        } else {
            0.0
        };

        // Ballistics.
        let env = self.env;
        let q = (if over > env { ita } else { itr }) * self.inv_sr * 1000.0;
        let coef = if q < 1.0 { q * LN2 } else { LN2 };
        let env_new = (1.0 - coef).mul_add(env, over * coef) + TINY;

        // The soft knee, from the previous sample's envelope and width.
        let wraw = -(k2 * t);
        let (w, y);
        if self.wraw > 0.0 {
            w = if wraw < W_MIN { W_MIN } else { wraw };
            let x = env * self.winv;
            y = if x > 1.0 {
                (x + -1.0).mul_add(2.0, 1.0) * (self.w * 0.5)
            } else if x > c(0x24e6_9595) {
                x * (x * (self.w * 0.5))
            } else {
                0.0
            };
        } else {
            w = W_MIN;
            y = env;
        }
        self.wraw = wraw;
        self.env = env_new;
        self.w = w;

        // The gain, with auto makeup.
        let a = if ta > 4.0 { 1.0 } else { ta * 0.25 };
        let h = a * (2.0 - a);
        let h2 = h * (2.0 - h);
        let m = (t * (s * 18.0 * c(0x3c88_8889))).mul_add(h2, gdb);
        let m = (s * c(0x3f75_c28f)).mul_add((k2 * 0.5).mul_add(t, -t), m);
        let g = e((-y).mul_add(s, m).mul_add(K_DB, 1200.0));
        self.reduction = y * s;

        // The output safety limiter, releasing at the attack time.
        let (dl, dr) = (self.dly[0].abs(), self.dly[1].abs());
        let pk = if dl < dr { dr } else { dl };
        let lim = if pk > self.lim {
            pk
        } else {
            let q = ita * self.inv_sr * 1000.0;
            let coef = if q < 1.0 { q * LN2 } else { LN2 };
            (pk - self.lim).mul_add(coef, self.lim) + TINY
        };
        self.lim = lim;
        let cl = if lim < 1.0 { 1.0 } else { lim };
        let ci = recip(if cl > 1000.0 { 1000.0 } else { cl });
        self.winv = recip(w);

        let out = (self.dly[0] * ci, self.dly[1] * ci);
        self.dly = [self.prev_in[0] * g, self.prev_in[1] * g];
        self.prev_in = [xl, xr];
        out
    }
}

// ---- Feedback (§5.2) -----------------------------------------------------------------

// Ramp lanes.
const F_KL: usize = 0;
const F_MK: usize = 1;
const F_KLX: usize = 2;
const F_R: usize = 3;
const F_KH: usize = 4;
const F_C: usize = 5;
const F_E: usize = 6;
const F_ATK: usize = 7;
const F_REL: usize = 8;
const F_THR: usize = 9;
const F_GAIN: usize = 10;
const F_ON: usize = 11;

struct Feedback {
    sr: f32,
    /// Normalized Attack and Release, for a sample-rate change.
    atk: f32,
    rel: f32,
    suspended: bool,
    ramp: Ramp<12>,
    t: f32,
    r: f32,
    kh: f32,
    kl: f32,
    neg_k: f32,
    ca: f32,
    cb: f32,
    b0: f32,
    b1: f32,
    a1: f32,
    h0: f32,
    h1: f32,
    ha: f32,
    // State.
    g: f32,
    m: f32,
    a: f32,
    b: f32,
    s1: [f32; 2],
    hp1: [f32; 2],
    xg1: [f32; 2],
    y1: [f32; 2],
}

impl Feedback {
    /// The core as `CompressorProcessor` makes it: `onInit`, then `onSetSampleRate(44100,
    /// 128)`, suspended.
    fn new() -> Feedback {
        let mut ramp = Ramp::new(44100.0);
        ramp.tgt[F_ON] = 1.0;
        ramp.cur[F_ON] = 1.0;
        let mut f = Feedback {
            sr: 44100.0,
            atk: c(0x3ea1_4517),
            rel: 0.5,
            suspended: true,
            ramp,
            t: 0.0,
            r: 1.0,
            kh: 1.0,
            kl: 1.0,
            neg_k: -0.5,
            ca: 0.0,
            cb: 0.0,
            b0: 0.0,
            b1: 0.0,
            a1: 0.0,
            h0: 0.0,
            h1: 0.0,
            ha: 0.0,
            g: 1.0,
            m: 0.0,
            a: 0.0,
            b: 0.0,
            s1: [0.0; 2],
            hp1: [0.0; 2],
            xg1: [0.0; 2],
            y1: [0.0; 2],
        };
        f.set_sample_rate(44100.0);
        f
    }

    /// `onSetSampleRate(sr, 128)`. The new Attack and Release coefficients become
    /// targets even while suspended, so they ramp in after resuming.
    fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr;
        let r = Ramp::<12>::new(sr);
        (self.ramp.ticks, self.ramp.inv) = (r.ticks, r.inv);
        self.ramp.tgt[F_ATK] = self.attack_coef(self.atk);
        self.ramp.tgt[F_REL] = self.release_coef(self.rel);
        self.ramp.dirty = true;
        let avg = 1.0 - e((-0.25 / (sr * c(0x3ef7_ced9))).mul_add(K_LN, 1200.0)); // τ = 0.484 s
        (self.ca, self.cb) = (avg, avg);
        // The output low-pass.
        let t = ((20000.0 / (sr + sr)).atan() * 2.0 * 0.5).tan();
        let k = c(0x4875_6fdb);
        let n = 1.0 / t.mul_add(k, 20000.0);
        self.b0 = t * k * n;
        self.b1 = self.b0;
        self.a1 = t.mul_add(k, -20000.0) * n;
        // The detector's 10 Hz high-pass.
        let kk = (c(0x41fb_53d2) / sr).tan();
        self.h0 = 1.0 / (kk + 1.0);
        self.h1 = -self.h0;
        self.ha = (kk + -1.0) * self.h0;
        self.reset();
    }

    fn target(&mut self, lane: usize, v: f32) {
        self.ramp.tgt[lane] = v;
        self.ramp.dirty = true;
        if self.suspended {
            self.ramp.cur = self.ramp.tgt;
        }
    }

    /// `1 − e^(−0.25/(τ·sr))`, the coefficient of a quarter-sample step.
    fn rc(&self, tau: f32) -> f32 {
        1.0 - e((-0.25 / (tau * self.sr)).mul_add(K_LN, 1200.0))
    }

    /// The makeup `10^((T/R − T)/40)`.
    fn makeup(&mut self) {
        let x = (self.t / self.r - self.t) * 0.5;
        self.target(F_MK, e(x.mul_add(K_DB, 1200.0)));
    }

    /// The knee's shape, from R and the knee's edges.
    fn shape(&mut self) {
        let l = self.kh.ln();
        let one_r = 1.0 - self.r;
        let cc = ((one_r as f64) / ((l as f64).powf(3.0) * 4.0)) as f32;
        let ee = (one_r * l * 0.25).exp();
        let sel = if cc < c(0xbdcc_cccd) { c(0xbc3c_a0a8) / cc } else { c(0x3deb_c8d2) };
        let klx = self.kl * sel.powf(0.25).exp();
        self.target(F_KL, self.kl);
        self.target(F_KLX, klx);
        self.target(F_R, self.r);
        self.target(F_KH, self.kh);
        self.target(F_C, cc);
        self.target(F_E, ee);
    }

    fn set_threshold(&mut self, v: f32) {
        self.t = v.mul_add(-60.0, 0.0);
        self.target(F_THR, e(self.t.mul_add(-K_DB, 1200.0)));
        self.makeup();
    }

    fn set_amount(&mut self, v: f32) {
        let x = v.mul_add(19.0, 1.0) + -1.0;
        self.r = x.mul_add(c(0x4019_702b), x.powf(4.0) * c(0x3887_6680)) + 1.0;
        self.makeup();
        self.shape();
    }

    fn set_knee(&mut self, v: f32) {
        let k = v.mul_add(4.5, 0.5);
        self.kh = e(k.mul_add(K_DB, 1200.0));
        self.neg_k = -k;
        self.kl = e(k.mul_add(-K_DB, 1200.0));
        self.shape();
    }

    fn attack_coef(&self, v: f32) -> f32 {
        self.rc((v * v * v).mul_add(49530.0, 470.0) * c(0x346c_3924))
    }

    fn release_coef(&self, v: f32) -> f32 {
        let r = e(v.mul_add(16.0, 32.0).mul_add(K_DB, 1200.0));
        let r = if r < 40.0 { 40.0 } else { r };
        let x = if r > 251.0 { 1.0 } else { (r + -40.0) / 211.0 };
        self.rc(x.mul_add(c(0x49a1_3e69), 270000.0) * c(0x346c_3924))
    }

    fn set_attack(&mut self, v: f32) {
        self.atk = v;
        self.target(F_ATK, self.attack_coef(v));
    }

    fn set_release(&mut self, v: f32) {
        self.rel = v;
        self.target(F_REL, self.release_coef(v));
    }

    fn set_gain(&mut self, v: f32) {
        self.target(F_GAIN, e(v.mul_add(36.0, -36.0).mul_add(K_DB, 1200.0)));
    }

    /// `onReset`.
    fn reset(&mut self) {
        self.s1 = [0.0; 2];
        self.hp1 = [0.0; 2];
        self.xg1 = [0.0; 2];
        self.y1 = [0.0; 2];
        self.g = 1.0;
        (self.m, self.a, self.b) = (self.neg_k, self.neg_k, self.neg_k);
    }

    /// `onReset` + `onResume`.
    fn resume(&mut self) {
        self.reset();
        self.ramp.tgt[F_ON] = 1.0;
        self.ramp.dirty = true;
        self.suspended = false;
    }

    /// The gain law at mean level `m`.
    fn law(m: f32, kl: f32, u: f32, cc: f32, ee: f32, one_r: f32) -> f32 {
        if m <= kl {
            1.0
        } else if m <= u {
            let t = (m / kl).ln();
            let t2 = t * t;
            e((t2 * (cc * t2)).mul_add(K_LN, 1200.0))
        } else {
            ee * e((one_r * (m / u).ln()).mul_add(K_LN, 1200.0))
        }
    }

    /// One sample of `FBCompressorCore::processAudio`, with the side-chain input if
    /// there is one.
    fn process(&mut self, il: f32, ir: f32, sc: Option<(f32, f32)>) -> (f32, f32) {
        self.ramp.step();
        let cur = self.ramp.cur;
        let (kl, r) = (cur[F_KL], cur[F_R]);
        let u = cur[F_KH] * kl;
        if r == 1.0 {
            // (sic: −k is a dB value, used here as a level.)
            self.g = 1.0;
            (self.m, self.a, self.b) = (self.neg_k, self.neg_k, self.neg_k);
        }
        // The detector: a 10 Hz high-pass of the input, stereo-linked peak.
        let x = [D + il, D + ir];
        let det = sc.map_or(x, |(sl, sr)| [D + sl, D + sr]);
        let mut hp = [0f32; 2];
        for ch in 0..2 {
            let s = det[ch];
            let y = (-self.ha).mul_add(self.hp1[ch], self.h0.mul_add(s, self.h1 * self.s1[ch]));
            self.s1[ch] = D + s;
            self.hp1[ch] = D + y;
            hp[ch] = y;
        }
        let (pl, pr) = (hp[0].abs(), hp[1].abs());
        let v = cur[F_THR] * if pl > pr { pl } else { pr };

        // Four quarter-sample steps of the feedback loop.
        let (atk, rel, klx, cc, ee) = (cur[F_ATK], cur[F_REL], cur[F_KLX], cur[F_C], cur[F_E]);
        let one_r = 1.0 - r;
        let (mut g, mut m, mut a, mut b) = (self.g, self.m, self.a, self.b);
        for _ in 0..4 {
            let p = v * g;
            let bb = if p < b { b } else { atk.mul_add(p - b, b) };
            a = D + self.cb.mul_add(m - a, a);
            b = D + self.ca.mul_add(m - bb, rel.mul_add(klx - bb, bb));
            m = (a + b) * 0.5;
            g = D + Self::law(m, kl, u, cc, ee, one_r);
        }
        (self.g, self.m, self.a, self.b) = (g, m, a, b);

        // The gain, a first-order low-pass, the output gain and the makeup.
        let mut out = [0f32; 2];
        for ch in 0..2 {
            let xg = x[ch] * g;
            let y = (-self.a1).mul_add(self.y1[ch], self.b0.mul_add(xg, self.b1 * self.xg1[ch]));
            self.xg1[ch] = D + xg;
            self.y1[ch] = D + y;
            out[ch] = y * cur[F_GAIN] * cur[F_ON] * cur[F_MK];
        }
        (out[0], out[1])
    }
}

// ---- The effect (§2) -----------------------------------------------------------------

/// Field names and defaults (normalized), in `tFloatParameters` order.
const PARAMS: [(&str, u32); 6] = [
    ("threshold", 0x3e2a_aaab),
    ("amount", 0x3f40_0000),
    ("knee", 0x3f00_0000),
    ("attack", 0x3ea1_4517),
    ("release", 0x3f00_0000),
    ("gain", 0x3f6a_aaab),
];

pub struct Compressor {
    sr: f32,
    mode: i64,
    classic: Classic,
    fb: Feedback,
    values: Option<[f32; 6]>,
    /// Samples to the Feedback core's next control tick (a grid across blocks).
    fb_ctr: usize,
    scf: SidechainFilter,
    /// The filtered side-chain block.
    sc: [Vec<f32>; 2],
}

impl Compressor {
    pub fn new(sr: f64) -> Compressor {
        let sr = sr as f32;
        Compressor {
            sr,
            mode: 0,
            classic: Classic::new(sr),
            fb: Feedback::new(),
            values: None,
            fb_ctr: 0,
            scf: SidechainFilter::new(sr as f64),
            sc: [Vec::new(), Vec::new()],
        }
    }

    /// `processFloatParameter`: both cores take every parameter.
    fn apply(&mut self, k: usize, v: f32) {
        let (c, f) = (&mut self.classic, &mut self.fb);
        match k {
            0 => (c.set_threshold(v), f.set_threshold(v)),
            1 => (c.set_amount(v), f.set_amount(v)),
            2 => (c.set_knee(v), f.set_knee(v)),
            3 => (c.set_attack(v), f.set_attack(v)),
            4 => (c.set_release(v), f.set_release(v)),
            _ => (c.set_gain(v), f.set_gain(v)),
        };
    }
}

impl Effect for Compressor {
    fn set(&mut self, params: &[(String, f64)]) {
        // Projects from before the Mode parameter (class v0) load as Classic.
        self.mode = param(params, "mode", 0.0) as i64;
        let new = PARAMS.map(|(name, def)| param(params, name, c(def) as f64) as f32);
        match self.values {
            None => {
                // The first settings apply at once: they reach the processor before it
                // resumes.
                for (k, v) in new.iter().enumerate() {
                    self.apply(k, *v);
                }
                // Then the host's sample rate, and the processor resumes.
                self.fb.set_sample_rate(self.sr);
                self.classic.resume();
                self.fb.resume();
                self.fb_ctr = 0;
            }
            Some(old) => {
                for k in 0..6 {
                    if new[k].to_bits() != old[k].to_bits() {
                        self.apply(k, new[k]);
                    }
                }
            }
        }
        self.values = Some(new);
        self.scf.set(params);
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.process_sc(l, r, None)
    }

    fn metered(&self) -> bool {
        true
    }

    /// As `Compressor::getGRMeterValue` reads it: the Classic core's reduction, or the
    /// Feedback core's gain.
    fn gain_reduction(&self) -> f32 {
        if self.mode == 0 { 10f32.powf(-self.classic.reduction.max(0.0) / 20.0) } else { self.fb.g.clamp(0.0, 1.0) }
    }

    /// `CompressorProcessor::process`: with a side-chain, the side-chain filter runs over
    /// it first, and the detectors take it instead of the input.
    fn process_sc(&mut self, l: &mut [f32], r: &mut [f32], sc: Option<(&[f32], &[f32])>) {
        let n = l.len().min(r.len());
        let [sl, sr] = &mut self.sc;
        let sc = sc.map(|(a, b)| {
            sl.clear();
            sl.extend_from_slice(&a[..n]);
            sr.clear();
            sr.extend_from_slice(&b[..n]);
            self.scf.process(sl, sr);
            (&sl[..], &sr[..])
        });
        let at = |i: usize| sc.map(|(a, b)| (a[i], b[i]));
        if self.mode == 0 {
            for (i, (xl, xr)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
                let x = (*xl, *xr);
                (*xl, *xr) = self.classic.process(x, at(i).unwrap_or(x));
            }
        } else {
            for (i, (xl, xr)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
                if self.fb_ctr == 0 {
                    self.fb.ramp.tick();
                    self.fb_ctr = 128;
                }
                self.fb_ctr -= 1;
                (*xl, *xr) = self.fb.process(*xl, *xr, at(i));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(mode: f64, threshold: f64, amount: f64, gain: f64) -> Vec<(String, f64)> {
        [("mode", mode), ("threshold", threshold), ("amount", amount), ("knee", 0.5), ("attack", 0.3), ("release", 0.5), ("gain", gain)]
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect()
    }

    fn run(fx: &mut Compressor, x: &[f32]) -> Vec<f32> {
        let (mut l, mut r) = (x.to_vec(), x.to_vec());
        for (a, b) in l.chunks_mut(300).zip(r.chunks_mut(300)) {
            fx.process(a, b);
        }
        l
    }

    fn sine(db: f32, n: usize) -> Vec<f32> {
        let a = 10f32.powf(db / 20.0);
        (0..n).map(|i| (i as f32 * 0.0573).sin() * a).collect()
    }

    /// The RMS of the last half, in dB.
    fn level(y: &[f32]) -> f32 {
        let t = &y[y.len() / 2..];
        10.0 * (t.iter().map(|v| v * v).sum::<f32>() / t.len() as f32).log10() + 3.0103
    }

    fn out_level(mode: f64, amount: f64, gain: f64, db: f32) -> f32 {
        let mut fx = Compressor::new(48_000.0);
        fx.set(&params(mode, 0.4, amount, gain)); // −24 dB threshold
        level(&run(&mut fx, &sine(db, 48_000)))
    }

    #[test]
    fn reduces_gain_above_the_threshold() {
        for mode in [0.0, 1.0] {
            // 30 dB more input (18 dB over the threshold) comes out well under 30 dB louder.
            let quiet = out_level(mode, 1.0, 1.0, -36.0);
            let loud = out_level(mode, 1.0, 1.0, -6.0);
            let rise = loud - quiet;
            assert!(rise < 20.0 && rise > 0.0, "mode {mode}: {rise} dB");
            // With Amount 0 nothing is compressed.
            let quiet = out_level(mode, 0.0, 1.0, -36.0);
            let loud = out_level(mode, 0.0, 1.0, -6.0);
            assert!((loud - quiet - 30.0).abs() < 0.1, "mode {mode}: {} dB", loud - quiet);
        }
    }

    /// The GR meter: no reduction under the threshold, about 9 dB for 18 dB over it at
    /// 2:1-ish settings, in both modes.
    #[test]
    fn reports_gain_reduction() {
        for mode in [0.0, 1.0] {
            let mut fx = Compressor::new(48_000.0);
            fx.set(&params(mode, 0.4, 1.0, 1.0));
            run(&mut fx, &sine(-40.0, 24_000));
            assert!(fx.metered());
            let quiet = -20.0 * fx.gain_reduction().log10();
            assert!(quiet.abs() < 0.5, "mode {mode}: {quiet} dB under the threshold");
            run(&mut fx, &sine(-6.0, 24_000));
            let loud = -20.0 * fx.gain_reduction().log10();
            assert!(loud > 3.0 && loud < 30.0, "mode {mode}: {loud} dB over it");
        }
    }

    #[test]
    fn makes_up_gain() {
        for mode in [0.0, 1.0] {
            // Below the threshold: unity without Amount, auto makeup with it.
            let flat = out_level(mode, 0.0, 1.0, -40.0);
            assert!((flat + 40.0).abs() < 0.1, "mode {mode}: {flat}");
            let made_up = out_level(mode, 0.8, 1.0, -40.0);
            assert!(made_up > -30.0, "mode {mode}: {made_up}");
            // Gain: −36·(1 − v) dB.
            let low = out_level(mode, 0.0, 0.5, -40.0);
            assert!((low + 58.0).abs() < 0.1, "mode {mode}: {low}");
        }
    }

    #[test]
    fn classic_is_two_samples_late() {
        let mut fx = Compressor::new(44_100.0);
        fx.set(&params(0.0, 0.0, 0.0, 1.0));
        let mut x = vec![0f32; 64];
        x[10] = 0.5;
        let y = run(&mut fx, &x);
        assert_eq!(y.iter().position(|v| *v != 0.0), Some(12));
        assert!((y[12] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn mode_switch_keeps_each_cores_state() {
        let x = sine(-6.0, 9600);
        let mut fx = Compressor::new(48_000.0);
        fx.set(&params(0.0, 0.4, 0.8, 0.9));
        let classic = run(&mut fx, &x);
        fx.set(&params(1.0, 0.4, 0.8, 0.9));
        let fb = run(&mut fx, &x);
        assert!(fb.iter().all(|v| v.is_finite()));
        assert!(classic.iter().zip(&fb).any(|(a, b)| (a - b).abs() > 1e-3), "the modes differ");
        // Back to Classic: it resumes where it stopped, so its output carries on as if
        // the Feedback block had not happened (its delay line holds the last input).
        let mut alone = Compressor::new(48_000.0);
        alone.set(&params(0.0, 0.4, 0.8, 0.9));
        run(&mut alone, &x);
        let a = run(&mut alone, &x);
        fx.set(&params(0.0, 0.4, 0.8, 0.9));
        let b = run(&mut fx, &x);
        assert_eq!(a, b);
    }

    #[test]
    fn parameter_changes_ramp() {
        let x = sine(-40.0, 4800);
        let mut fx = Compressor::new(48_000.0);
        fx.set(&params(0.0, 0.4, 0.0, 1.0));
        run(&mut fx, &x);
        fx.set(&params(0.0, 0.4, 0.0, 0.0)); // 0 → −36 dB over ≈ 20 ms
        let y = run(&mut fx, &x);
        let peak = |s: &[f32]| s.iter().fold(0f32, |m, v| m.max(v.abs()));
        let early = peak(&y[..200]);
        let late = peak(&y[2000..]);
        assert!(early > 0.009, "{early}");
        assert!((late / 0.01 - 10f32.powf(-36.0 / 20.0)).abs() < 1e-3, "{late}");
    }

    #[test]
    fn exp_table() {
        assert_eq!(e(1200.0), 1.0);
        assert!((e(1200.0 + 60.0) - 2.0).abs() < 1e-6);
        assert!((e((-6.0f32).mul_add(K_DB, 1200.0)) - 0.501_187).abs() < 1e-4);
        assert_eq!(e(-1.5), 0.0);
        assert_eq!(e(1e9), 1_048_576.0);
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn frecpe_matches_neon() {
        use std::arch::aarch64::*;
        for i in 0..20_000 {
            let x = 0.0007 * 1.0007f32.powi(i);
            // SAFETY: NEON is always present on aarch64.
            let hw = unsafe { vget_lane_f32::<0>(vrecpe_f32(vdup_n_f32(x))) };
            assert_eq!(frecpe(x).to_bits(), hw.to_bits(), "{x}");
        }
    }
}
