//! Maschine's Limiter (docs/fx-limiter.md): Legacy (`FXCORE::LimiterCore`, §3) and
//! Transparent (`NI::DSP::HilbertLimiter`, §4), chosen by Mode. Only the chosen engine
//! runs; the other keeps its state. With a side-chain source, the side-chain filter (§5)
//! shapes it and the detectors see it.

use super::sidechain::SidechainFilter;
use super::{Effect, param};

/// `10^x` as Apple's libm computes it (Maschine's `exp10f`).
fn exp10f(x: f32) -> f32 {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn __exp10f(x: f32) -> f32;
        }
        unsafe { __exp10f(x) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        10f32.powf(x)
    }
}

pub struct Limiter {
    mode: i64,
    legacy: Legacy,
    hilbert: Hilbert,
    scf: SidechainFilter,
    /// The filtered side-chain block.
    sc: [Vec<f32>; 2],
    /// Threshold and (Transparent) ceiling in dB, for the GR meter.
    thr_db: f32,
    ceil_db: f32,
    /// The last block's gain reduction (linear), for the meter.
    gr: f32,
}

impl Limiter {
    pub fn new(sr: f64) -> Limiter {
        Limiter {
            mode: 1,
            legacy: Legacy::new(sr as f32),
            hilbert: Hilbert::new(sr as f32),
            scf: SidechainFilter::new(sr),
            sc: [Vec::new(), Vec::new()],
            thr_db: 0.0,
            ceil_db: 0.0,
            gr: 1.0,
        }
    }
}

impl Effect for Limiter {
    fn set(&mut self, params: &[(String, f64)]) {
        // Projects from before the Mode parameter (class v0) load as Legacy.
        self.mode = param(params, "mode", 0.0) as i64;
        let thr = param(params, "threshold", 0.0) as f32;
        self.thr_db = thr;
        self.ceil_db = (1.0 - param(params, "ceiling", 1.0) as f32) * -39.7 - 0.35;
        self.legacy.set_threshold(thr);
        self.hilbert.set_threshold(thr);
        self.hilbert.set_release(param(params, "release", 0.0) as f32);
        self.hilbert.set_ceiling(param(params, "ceiling", 1.0) as f32);
        // The first settings apply at once (the engines start suspended).
        self.legacy.active = true;
        self.hilbert.resume();
        self.scf.set(params);
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.process_sc(l, r, None)
    }

    /// `LimiterProcessor::process`: with a side-chain, the side-chain filter runs over it
    /// first, and the detector takes it instead of the input.
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
        // The meter's reduction: how far the driven peak (threshold as drive) goes over
        // the ceiling (0 dBFS in Legacy). An estimate per block; the DSP is untouched.
        let peak = sc.map_or((&l[..n], &r[..n]), |(a, b)| (a, b));
        let p = peak.0.iter().chain(peak.1).fold(0f32, |m, x| m.max(x.abs()));
        let ceil = if self.mode == 1 { self.ceil_db } else { 0.0 };
        let over = if p > 0.0 { 20.0 * p.log10() - self.thr_db - ceil } else { 0.0 };
        self.gr = 10f32.powf(-over.max(0.0) / 20.0);
        if self.mode == 1 { self.hilbert.process(l, r, sc) } else { self.legacy.process(l, r, sc) }
    }

    fn metered(&self) -> bool {
        true
    }

    fn gain_reduction(&self) -> f32 {
        self.gr
    }
}

// ---- Legacy (§3) ---------------------------------------------------------------------

struct Legacy {
    n: usize,
    l: usize,
    w: (f32, f32),
    s_half: f32,
    c_half: f32,
    k_thr: f32,
    k_att: f32,
    k_rel: f32,
    tree: Vec<f32>,
    tree_w: usize,
    d: Vec<f32>,
    di: usize,
    a: Vec<f32>,
    ai: usize,
    acc: [f32; 4],
    ctr: usize,
    flip: bool,
    z: (f32, f32),
    env: f32,
    g: f32,
    gain_target: f32,
    active: bool,
}

impl Legacy {
    fn new(sr: f32) -> Legacy {
        let n = ((sr as f64 * 88.0 / 44100.0) + 0.5) as usize;
        let n = n.max(1);
        // The smallest power of two ≥ N (at least 2).
        let mut x = 2usize;
        let mut prev;
        loop {
            prev = x;
            x *= 2;
            if prev >= n {
                break;
            }
        }
        let l = prev;
        let theta = std::f32::consts::PI / n as f32;
        // Maschine's literal ln 2 (0x3f317215), not f32's LN_2 (one ulp apart).
        const LN2: f32 = f32::from_bits(0x3f31_7215);
        let k = |t: f32| {
            let s = sr * t;
            if s >= 1.0 { LN2 / s } else { LN2 }
        };
        Legacy {
            n,
            l,
            w: (theta.cos(), -theta.sin()),
            s_half: (theta / 2.0).sin(),
            c_half: (theta / 2.0).cos(),
            k_thr: k(0.01),
            k_att: k(0.000_66),
            k_rel: k(0.2),
            tree: vec![1.0; l],
            tree_w: 0,
            d: vec![0.0; l],
            di: 0,
            a: vec![0.0; 2 * l],
            ai: 0,
            acc: [0.0; 4],
            ctr: 1,
            flip: false,
            z: (1.0, 0.0),
            env: 0.0,
            g: 1.0,
            gain_target: 1.0,
            active: false,
        }
    }

    fn set_threshold(&mut self, t: f32) {
        self.gain_target = (t * -0.115_129_25).exp();
        if !self.active {
            self.g = self.gain_target;
        }
    }

    fn process(&mut self, ls: &mut [f32], rs: &mut [f32], sc: Option<(&[f32], &[f32])>) {
        let (n, mask_d, mask_a) = (self.n, self.l - 1, 2 * self.l - 1);
        for (i, (xl, xr)) in ls.iter_mut().zip(rs.iter_mut()).enumerate() {
            self.g = self.k_thr.mul_add(self.gain_target - self.g, self.g);
            let (al, ar) = (*xl * self.g, *xr * self.g);
            // The detector: the side-chain driven by the same gain, or the driven input.
            let (dl, dr) = sc.map_or((al, ar), |(sl, sr)| (sl[i] * self.g, sr[i] * self.g));
            let p = dl.abs().max(dr.abs()).max(0.0);
            let dlt = p - self.env;
            // The release coefficient is stored as kRel − kAtt and added back to kAtt.
            let k = if dlt > 0.0 { self.k_att } else { (self.k_rel - self.k_att) + self.k_att };
            self.env = dlt.mul_add(k, self.env);
            let g1 = if self.env < 1.0 { 1.0 } else { 1.0 / self.env };
            self.a[self.ai] = al * g1;
            self.a[self.ai + 1] = ar * g1;
            // Sliding maximum of p over the last N samples (slots past N stay 1.0).
            self.tree[self.tree_w] = p;
            self.tree_w = if self.tree_w + 1 >= n { 0 } else { self.tree_w + 1 };
            let m = self.tree.iter().copied().fold(f32::MIN, f32::max);
            let y = m * self.s_half;
            self.d[self.di] = y;
            // Sine-window FIR by a rotating phasor, re-anchored every N samples.
            if self.ctr < 2 {
                self.ctr = n;
                self.z = if self.flip { (-1.0, -0.0) } else { (1.0, 0.0) };
                self.flip = !self.flip;
                self.acc = [self.acc[2], self.acc[3], 0.0, 0.0];
            } else {
                self.ctr -= 1;
                let (zr, zi) = self.z;
                let (wr, wi) = self.w;
                self.z = (zr * wr - zi * wi, zr * wi + zi * wr);
            }
            let x = self.d[(self.di + self.l - n) & mask_d];
            self.di = (self.di + 1) & mask_d;
            let (zr, zi) = self.z;
            self.acc[0] += zr * (y + x);
            self.acc[1] += zi * (y + x);
            self.acc[2] += zr * y;
            self.acc[3] += zi * y;
            let (a0, a1) = (self.acc[0], self.acc[1]);
            let re = a0 * zr + a1 * zi;
            let im = a1 * zr - a0 * zi;
            let e = self.c_half * im + self.s_half * re;
            let gain = 1.0 / e;
            let at = (self.ai + 2 * self.l - 2 * n) & mask_a;
            let (ol, or) = (self.a[at] * gain, self.a[at + 1] * gain);
            self.ai = (self.ai + 2) & mask_a;
            *xl = if ol.is_finite() { ol } else { 0.0 };
            *xr = if or.is_finite() { or } else { 0.0 };
        }
    }
}

// ---- Transparent (§4) ----------------------------------------------------------------

/// The Hilbert all-pass coefficients (§4.3), as Maschine's NEON reciprocal gives them:
/// per stage, (lanes 0 and 2, lanes 1 and 3).
const HILBERT: [(u32, u32); 7] = [
    (0xbf4b104a, 0xbf20b130),
    (0xbebf656b, 0xbd1e9e7d),
    (0x3eae8a20, 0x3f40b752),
    (0xbf71639d, 0xbf78872d),
    (0xbf7c3917, 0xbf7e21fc),
    (0xbf7f261f, 0xbf7fc16a),
    (0xbf63d70a, 0xbf63d70a),
];

/// Seven first-order all-pass stages over (dL, dL, dR, dR).
struct HilbertTransformer {
    a: [[f32; 4]; 7],
    ring: [[[f32; 4]; 8]; 7],
    idx: [usize; 7],
    m: usize,
}

impl HilbertTransformer {
    fn new(m: usize) -> HilbertTransformer {
        let mut a = [[0f32; 4]; 7];
        for (k, (even, odd)) in HILBERT.iter().enumerate() {
            let (e, o) = (f32::from_bits(*even), f32::from_bits(*odd));
            a[k] = [e, o, e, o];
        }
        HilbertTransformer { a, ring: [[[0.0; 4]; 8]; 7], idx: [0; 7], m: m.clamp(1, 8) }
    }

    /// (L real, L quadrature, R real, R quadrature).
    fn process(&mut self, l: f32, r: f32) -> [f32; 4] {
        let mut u = [l, l, r, r];
        let mut y6 = [0f32; 4];
        for st in 0..7 {
            let idx = self.idx[st];
            let s = self.ring[st][(idx + 8 + 1 - self.m) & 7];
            let mut y = [0f32; 4];
            let mut w = [0f32; 4];
            for k in 0..4 {
                let t = self.a[st][k] * (u[k] - s[k]);
                w[k] = u[k] + t;
                y[k] = s[k] + t;
            }
            self.ring[st][(idx + 1) & 7] = w;
            self.idx[st] = (idx + 1) & 7;
            if st == 5 {
                y6 = y;
            }
            u = y;
        }
        [y6[0], u[1], y6[2], u[3]]
    }
}

/// A recursive periodic-Hann FIR of length N with unity DC gain (§4.5), in double.
struct HannFilter {
    n: usize,
    inv_n: f64,
    c: f64,
    c2: f64,
    buf: Vec<f64>,
    mask: usize,
    i: usize,
    s: f64,
    y1: f64,
    y2: f64,
    s_sh: f64,
    y1_sh: f64,
    y2_sh: f64,
    p: usize,
    cnt: usize,
    warm: usize,
}

impl HannFilter {
    fn new(n: usize, switch_rate: usize) -> HannFilter {
        let n = n.max(1);
        let bits = usize::BITS - n.leading_zeros();
        let size = 1usize << bits;
        let inv_n = 1.0 / n as f64;
        let c = (inv_n * std::f64::consts::TAU).cos();
        let p = n.max(switch_rate);
        HannFilter {
            n,
            inv_n,
            c,
            c2: c + c,
            buf: vec![0.0; size],
            mask: size - 1,
            i: 0,
            s: 0.0,
            y1: 0.0,
            y2: 0.0,
            s_sh: 0.0,
            y1_sh: 0.0,
            y2_sh: 0.0,
            p,
            cnt: 0,
            warm: p - n,
        }
    }

    fn process(&mut self, x: f64) -> f64 {
        self.buf[self.i] = x;
        let d = x - self.buf[(self.i + self.buf.len() - self.n) & self.mask];
        self.i = (self.i + 1) & self.mask;
        self.s += d;
        let r = d + self.y1 * self.c2 - self.y2;
        let y1o = self.y1;
        self.y2 = self.y1;
        self.y1 = r;
        let out = self.inv_n * (self.s - (r - y1o * self.c));
        let was = self.cnt;
        self.cnt += 1;
        if was >= self.warm {
            let rs = x + self.y1_sh * self.c2 - self.y2_sh;
            self.y2_sh = self.y1_sh;
            self.y1_sh = rs;
            self.s_sh += x;
            if self.cnt >= self.p {
                (self.y1, self.y2, self.s) = (self.y1_sh, self.y2_sh, self.s_sh);
                (self.y1_sh, self.y2_sh, self.s_sh) = (0.0, 0.0, 0.0);
                self.cnt = 0;
            }
        }
        out
    }
}

/// Two linearly ramped values (threshold drive, ceiling), updated per 128-sample
/// control block (§4.6).
struct Smoother {
    cur: [f32; 2],
    start: [f32; 2],
    target: [f32; 2],
    delta: [f32; 2],
    pending: Option<[f32; 2]>,
    ctr: f32,
    inv: f32,
    k: usize,
    left: usize,
    suspended: bool,
}

impl Smoother {
    fn latch(&mut self, v: [f32; 2]) {
        self.target = v;
        self.start = self.cur;
        self.ctr = 0.0;
        let mut any = false;
        for k in 0..2 {
            self.delta[k] = self.target[k] - self.cur[k];
            // Too small to move: snap.
            let ulp = f32::from_bits(self.cur[k].abs().to_bits() + 1) - self.cur[k].abs();
            if self.delta[k].abs() < ulp {
                self.delta[k] = 0.0;
                self.cur[k] = self.target[k];
                self.start[k] = self.target[k];
            } else {
                any = true;
            }
        }
        self.left = if any { self.k } else { 0 };
    }

    fn control(&mut self) {
        if self.left > 0 {
            self.left -= 1;
            if self.left == 0 {
                self.cur = self.target;
                self.start = self.target;
                self.delta = [0.0; 2];
                if let Some(v) = self.pending.take() {
                    self.latch(v);
                }
            }
        } else if let Some(v) = self.pending.take() {
            self.latch(v);
        }
    }

    fn step(&mut self) {
        if self.delta != [0.0; 2] {
            self.ctr += 1.0;
            let f = self.ctr * self.inv;
            for k in 0..2 {
                self.cur[k] = self.start[k] + self.delta[k] * f;
            }
        }
    }

    fn set(&mut self, lane: usize, v: f32) {
        if self.suspended {
            self.cur[lane] = v;
            self.start[lane] = v;
            self.target[lane] = v;
            self.delta[lane] = 0.0;
            if let Some(p) = &mut self.pending {
                p[lane] = v;
            }
        } else {
            let mut p = self.pending.unwrap_or(self.target);
            p[lane] = v;
            self.pending = Some(p);
        }
    }
}

struct Hilbert {
    sr: f32,
    w: usize,
    sm: Smoother,
    c_atk_h: f32,
    c_rel_h: f32,
    c06: f32,
    c_rel: f32,
    hilbert: HilbertTransformer,
    hann: HannFilter,
    al: Vec<f32>,
    ar: Vec<f32>,
    dbl: Vec<f32>,
    dbr: Vec<f32>,
    ia: usize,
    ib: usize,
    env_l: f32,
    env_r: f32,
    r: f32,
    s: f32,
    hold_l: f32,
    hold_r: f32,
    cnt_l: usize,
    cnt_r: usize,
    block: usize,
}

impl Hilbert {
    fn new(sr: f32) -> Hilbert {
        let m = ((sr / 44100.0) as usize).max(1);
        let w = 128 * m;
        let k = ((sr * 0.02 / 128.0 + 0.5) as usize).max(1);
        let c = |t: f32| 1.0 - 1.0 / (t * sr);
        Hilbert {
            sr,
            w,
            sm: Smoother {
                cur: [1.0; 2],
                start: [1.0; 2],
                target: [1.0; 2],
                delta: [0.0; 2],
                pending: None,
                ctr: 0.0,
                inv: 1.0 / (k * 128) as f32,
                k,
                left: 0,
                suspended: true,
            },
            c_atk_h: c(0.014),
            c_rel_h: c(0.003),
            c06: c(0.0006),
            c_rel: c(0.01),
            hilbert: HilbertTransformer::new(m),
            hann: HannFilter::new(w, sr as usize),
            al: vec![0.0; 2 * w],
            ar: vec![0.0; 2 * w],
            dbl: vec![0.0; w],
            dbr: vec![0.0; w],
            ia: 0,
            ib: 0,
            env_l: 0.0,
            env_r: 0.0,
            r: 0.0,
            s: 0.0,
            hold_l: -100.0,
            hold_r: -100.0,
            cnt_l: 0,
            cnt_r: 0,
            block: 0,
        }
    }

    fn set_threshold(&mut self, t: f32) {
        let v = (t + 40.0) / 40.0;
        let thr_db = (1.0 - v) * 40.0;
        self.sm.set(0, exp10f(thr_db / 20.0));
    }

    fn set_release(&mut self, v: f32) {
        let rel = ((v * 6.214_608).exp() / 500.0).max(0.001);
        self.c_rel = 1.0 - 1.0 / (rel * self.sr);
    }

    fn set_ceiling(&mut self, v: f32) {
        let db = (1.0 - v) * -39.7 + -0.35;
        self.sm.set(1, exp10f(db / 20.0));
    }

    fn resume(&mut self) {
        self.sm.suspended = false;
    }

    /// The peak hold of one channel's dB ring at the sample just written (§4.4).
    fn hold(db: &[f32], ib: usize, hold: &mut f32, cnt: &mut usize, w: usize) {
        let v = db[ib];
        if v >= *hold {
            *hold = v;
            *cnt = w - 1;
        } else if *cnt != 0 {
            *cnt -= 1;
        } else {
            *hold = -100.0;
            let mut pos = 0;
            for j in 0..w {
                let x = db[(ib + w - j) & (w - 1)];
                if x > *hold {
                    *hold = x;
                    pos = j;
                }
            }
            *cnt = w - 1 - pos;
        }
    }

    fn process(&mut self, ls: &mut [f32], rs: &mut [f32], sc: Option<(&[f32], &[f32])>) {
        let (w, ma, mb) = (self.w, 2 * self.w - 1, self.w - 1);
        for (i, (xl, xr)) in ls.iter_mut().zip(rs.iter_mut()).enumerate() {
            if self.block == 0 {
                self.sm.control();
                self.block = 128;
            }
            self.block -= 1;
            self.sm.step();
            let (gt, gc) = (self.sm.cur[0], self.sm.cur[1]);
            // With a side-chain the audio isn't driven, the side-chain is.
            let ga = if sc.is_some() { 1.0 } else { gt };
            let (sl, sr) = sc.map_or((*xl, *xr), |(a, b)| (a[i], b[i]));
            self.al[self.ia] = xl.mul_add(ga, 1e-12);
            self.ar[self.ia] = xr.mul_add(ga, 1e-12);
            let dl = sl.mul_add(gt, 1e-12);
            let dr = sr.mul_add(gt, 1e-12);
            self.dbl[self.ib] = 20.0 * dl.abs().log10();
            self.dbr[self.ib] = 20.0 * dr.abs().log10();
            let h = self.hilbert.process(dl, dr);
            let el = h[0].hypot(h[1]);
            self.env_l = if self.env_l < el { self.c_atk_h } else { self.c_rel_h }.mul_add(self.env_l - el, el);
            let er = h[2].hypot(h[3]);
            self.env_r = if self.env_r < er { self.c_atk_h } else { self.c_rel_h }.mul_add(self.env_r - er, er);
            let lvl = 20.0 * self.env_l.max(self.env_r).max(dl.abs()).max(dr.abs()).log10();
            self.r = if self.r < lvl { lvl } else { self.c_rel.mul_add(self.r - lvl, lvl) };
            self.s = self.c06.mul_add(self.s - self.r, self.r);
            let mut gdb = (0.0 - self.s).min(0.0);
            Self::hold(&self.dbl, self.ib, &mut self.hold_l, &mut self.cnt_l, w);
            if gdb + self.hold_l > 0.0 {
                gdb = 0.0 - self.hold_l;
            }
            Self::hold(&self.dbr, self.ib, &mut self.hold_r, &mut self.cnt_r, w);
            if gdb + self.hold_r > 0.0 {
                gdb = 0.0 - self.hold_r;
            }
            let glin = exp10f(gdb / 20.0);
            let hann = self.hann.process(glin as f64) as f32;
            let k = gc * hann;
            let at = (self.ia + 2 * w - (w - 1)) & ma;
            *xl = k * self.al[at];
            *xr = k * self.ar[at];
            self.ia = (self.ia + 1) & ma;
            self.ib = (self.ib + 1) & mb;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(mode: f64, threshold: f64) -> Vec<(String, f64)> {
        vec![("mode".into(), mode), ("threshold".into(), threshold), ("release".into(), 0.3), ("ceiling".into(), 1.0)]
    }

    fn run(fx: &mut Limiter, x: &[f32]) -> Vec<f32> {
        let (mut l, mut r) = (x.to_vec(), x.to_vec());
        for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            fx.process(a, b);
        }
        l
    }

    #[test]
    fn transparent_holds_the_ceiling_and_delays() {
        let sr = 48_000.0;
        let mut fx = Limiter::new(sr);
        fx.set(&params(1.0, -12.0)); // 12 dB of drive into a −0.35 dB ceiling
        let x: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.05).sin() * 0.9).collect();
        let y = run(&mut fx, &x);
        let ceiling = 10f32.powf(-0.35 / 20.0);
        let peak = y[4800..].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak <= ceiling * 1.0001, "peak {peak} over the ceiling {ceiling}");
        assert!(peak > ceiling * 0.95, "limited up to the ceiling: {peak}");
        // A quiet signal passes, delayed W − 1 = 127 samples.
        let mut fx = Limiter::new(sr);
        fx.set(&params(1.0, 0.0));
        let mut x = vec![0f32; 2000];
        x[10] = 0.1;
        let y = run(&mut fx, &x);
        let at = y.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert_eq!(at, 10 + 127);
    }

    #[test]
    fn legacy_limits_and_delays() {
        let sr = 44_100.0;
        let mut fx = Limiter::new(sr);
        fx.set(&params(0.0, -6.0));
        let x: Vec<f32> = (0..44_100).map(|i| (i as f32 * 0.03).sin() * 0.5).collect();
        let y = run(&mut fx, &x);
        let peak = y[8000..].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak <= 1.0, "{peak}");
        // Quiet: passes, delayed N = 88 samples.
        let mut fx = Limiter::new(sr);
        fx.set(&params(0.0, 0.0));
        let mut x = vec![0f32; 2000];
        x[10] = 0.1;
        let y = run(&mut fx, &x);
        let at = y.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert_eq!(at, 10 + 88);
        assert!((y[at] - 0.1).abs() < 1e-3, "{}", y[at]);
    }
}
