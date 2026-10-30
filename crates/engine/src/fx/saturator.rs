//! Maschine's Saturator (docs/fx-saturator.md): three modes, each its own core, chosen by
//! Mode. Classic is `FXCORE::SaturatorCore` (§4), Tape is `NI::DSP::Tape` (§5) and Tube is
//! `NI::DSP::WarmTube` (§6). Every core always takes its own parameters; only the chosen
//! one runs, and the others keep their state (§3).

use super::compressor::e;
use super::{Effect, param};

mod gain_table;
mod tape;

use tape::Tape;

/// An `f32` constant as Maschine stores it.
const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}
const K_DB: f32 = c(0x411f_73da); // 60 / (20·log10 2): dB → ExpTable index
const SECS_20MS: f32 = c(0x3ca3_d70a); // 0.02

/// The parameter ramps (§3.2) over `N` lanes: linear, `ticks` control ticks of 128 samples.
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
    /// A ramp `secs` long (as an `f32` constant) at `sr`.
    fn new(sr: f32, secs: f32) -> Self {
        let mut r = Ramp { cur: [0.0; N], inc: [0.0; N], tgt: [0.0; N], dirty: false, countdown: 0, ticks: 1, inv: 1.0 };
        r.set_sample_rate(sr, secs);
        r
    }

    fn set_sample_rate(&mut self, sr: f32, secs: f32) {
        self.ticks = (((sr * secs) / 128.0 + 0.5) as i32).max(1);
        self.inv = 1.0 / (self.ticks * 128) as f32;
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

    /// Jump to the targets (a core's init); the next tick still sees a change.
    fn snap(&mut self) {
        self.cur = self.tgt;
        self.inc = [0.0; N];
        self.dirty = true;
    }
}

/// `x` clamped to ±`lim` (NEON `fmin` then `fmax`).
fn clamp(x: f32, lim: f32) -> f32 {
    x.min(lim).max(-lim)
}

// ---- Classic: FXCORE::SaturatorCore (§4) ------------------------------------------------

// Ramp lanes.
const S_CG: usize = 0; // Input makeup gain
const S_Y: usize = 1; // high-band drive share
const S_X: usize = 2; // drive shape (Contour)
const S_Z: usize = 3; // wet mix
const S_D: usize = 4; // drive

struct Classic {
    sr: f32,
    /// The stored Input, Drive and Contour (0..1).
    raw: [f32; 3],
    /// Input as set (+0x34), drive `v(2−v)` (+0x3c), Contour (+0x40), drive slope (+0x50).
    cmp: f32,
    drive: f32,
    contour: f32,
    slope: f32,
    ramp: Ramp<8>,
    tick: i32,
    /// Envelope coefficients (+0x54, +0x58, +0x64, +0x68).
    c_env: f32,
    c_gain: f32,
    c_up: f32,
    c_down: f32,
    /// The band-split one-pole: `K = tan(1727.876/sr)`, `1/(K+1)`, `K−1`.
    k: f32,
    a0inv: f32,
    km1: f32,
    /// The Input stage's smoothed gain (+0x5c) and envelope (+0x60); the drive envelope (+0x6c).
    gsm: f32,
    env: f32,
    env2: f32,
    /// One-pole state per channel (+0x80).
    lp: [f32; 2],
    /// The band signals of the last three samples (+0x90, +0xa0, +0xb0), lanes
    /// `[hiL, hiR, loL, loR]`.
    u1: [f32; 4],
    u2: [f32; 4],
    u3: [f32; 4],
    /// The previous sample after the Input stage (+0xc0).
    dry: [f32; 2],
}

impl Classic {
    /// The constructor: defaults, `beginInit(44100)`, `endInit()`.
    fn new() -> Classic {
        let mut s = Classic::zeroed(44_100.0, [c(0x3e80_0000), c(0x3f00_0000), c(0x3f40_0000)]);
        s.init();
        s
    }

    fn zeroed(sr: f32, raw: [f32; 3]) -> Classic {
        Classic {
            sr,
            raw,
            cmp: 0.0,
            drive: 0.0,
            contour: 0.0,
            slope: 0.0,
            ramp: Ramp::new(sr, SECS_20MS),
            tick: 129,
            c_env: 0.0,
            c_gain: 0.0,
            c_up: 0.0,
            c_down: 0.0,
            k: 0.0,
            a0inv: 0.0,
            km1: 0.0,
            gsm: 0.0,
            env: 0.0,
            env2: 0.0,
            lp: [0.0; 2],
            u1: [0.0; 4],
            u2: [0.0; 4],
            u3: [0.0; 4],
            dry: [0.0; 2],
        }
    }

    /// `beginInit(sr)` then `endInit()` with the stored parameters (the constructor and
    /// `SaturatorProcessor::resume`): all state cleared, the ramps at their targets.
    fn init(&mut self) {
        *self = Classic::zeroed(self.sr, self.raw);
        let ms = self.sr * c(0x3a83_126f);
        // 1 − 2^(n / (sr·0.001·m − 1)), or 1 − 2^n when the denominator is below 1.
        let coef = |m: u32, n: u32| {
            let d = ms.mul_add(c(m), -1.0);
            let x = if 1.0 > d { c(n) } else { c(n) / d };
            1.0 - x.exp2()
        };
        self.c_env = coef(0x412d_6bba, 0xc054_9a78); // 10.8388, −log2(10)
        self.c_gain = coef(0x402b_707a, 0xc0d4_9a78); // 2.67874, −2·log2(10)
        self.c_up = coef(0x4000_0000, 0xc054_9a78); // 2
        self.c_down = coef(0x4220_0000, 0xc054_9a78); // 40
        self.k = (c(0x44d7_fc08) / self.sr).tan();
        self.a0inv = 1.0 / (self.k + 1.0);
        self.km1 = self.k + -1.0;
        self.env = c(0x3dcc_cccd);
        let raw = self.raw;
        self.set_input(raw[0]);
        self.set_drive(raw[1]);
        self.set_contour(raw[2]);
        self.ramp.snap();
    }

    /// `processEventInCmp`: Input.
    fn set_input(&mut self, v: f32) {
        self.raw[0] = v;
        let q = (1.0 - v) * (1.0 - v);
        let g = (q * q - q).mul_add(v, q);
        self.cmp = v;
        self.ramp.tgt[S_CG] = g.mul_add(c(0x3edf_6b7f), c(0x3f10_4a41));
        self.ramp.dirty = true;
    }

    /// The Contour share: `d/4 + contour·((0.9 − 0.15·slope) − d/4)`.
    fn shape(&self) -> f32 {
        let q = self.drive * 0.25;
        (self.slope.mul_add(c(0xbe19_999a), c(0x3f66_6666)) - q).mul_add(self.contour, q)
    }

    /// `processEventInDrv`: Drive.
    fn set_drive(&mut self, v: f32) {
        self.raw[1] = v;
        let d = v * (2.0 - v);
        self.drive = d;
        self.slope = if d > 0.0 { (d + c(0xbf4c_cccd)) * 5.0 } else { 0.0 };
        let z = d * 25.0;
        let x = self.shape();
        let t = &mut self.ramp.tgt;
        t[S_D] = d;
        t[S_Y] = self.slope.mul_add(c(0xbe47_ae14), c(0x3f26_6666));
        t[S_X] = x;
        t[S_Z] = if z > 1.0 { 1.0 } else { z };
        self.ramp.dirty = true;
    }

    /// `processEventInCntr`: Contour.
    fn set_contour(&mut self, v: f32) {
        self.raw[2] = v;
        self.contour = v;
        self.ramp.tgt[S_X] = self.shape();
        self.ramp.dirty = true;
    }

    /// `processAudio` for one stereo sample.
    fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.tick += 1;
        if self.tick >= 128 {
            self.tick = 0;
            self.ramp.tick();
        }
        self.ramp.step();
        let cur = self.ramp.cur;
        let (mut l, mut r) = (l, r);

        // Input: a peak compressor through the gain curve table.
        if self.cmp > 0.0 {
            let (al, ar) = (l.abs(), r.abs());
            let p = if al < ar { ar } else { al };
            let d = p - self.env;
            self.env = if d > 0.0 { p } else { d.mul_add(self.c_env, self.env) };
            let mut x = self.env + c(0x337f_fff3);
            if x > 1.0 {
                x = 1.0;
            }
            let xi = x * 127.0;
            let row = (self.cmp * 50.0).ceil() as i32;
            let f = xi + -0.5;
            let i = ((f as i32) + row * 128).clamp(0, 6527) as usize;
            let t0 = c(gain_table::GAIN[i]);
            let t1 = c(gain_table::GAIN[i + 1]);
            let g = (t1 - t0).mul_add(xi - (f as i32) as f32, t0) / x;
            self.gsm = (g - self.gsm).mul_add(self.c_gain, self.gsm);
            let s = cur[S_CG] * self.gsm;
            l *= s;
            r *= s;
        }

        // The drive envelope.
        let (al, ar) = (l.abs(), r.abs());
        let p = (if al < ar { ar } else { al }) - self.env2;
        let k = if p > 0.0 { self.c_up } else { self.c_down };
        self.env2 = (p + c(0x2edb_e6ff)).mul_add(k, self.env2);
        let m = if self.env2 > 1.0 { 1.0 } else { self.env2 };
        let a = cur[S_D] * (1.0 - m).mul_add(cur[S_X], m);
        let b = cur[S_Y] * a;

        // Split into a low band (one-pole) and the rest; drive each in dB.
        for (y, x) in self.lp.iter_mut().zip([l, r]) {
            *y = (*y + (x - *y * self.km1) * self.a0inv) * self.k;
        }
        let ga = e((a * 60.0).mul_add(K_DB, 1200.0));
        let gb = e((b * 60.0).mul_add(K_DB, 1200.0));
        let u = [(l - self.lp[0]) * gb, (r - self.lp[1]) * gb, self.lp[0] * ga, self.lp[1] * ga];

        // Drive-dependent output scale per band.
        let w = |v: f32| {
            let q = (1.0 - v) * (1.0 - v);
            let q = q * q;
            (q + (q * q - q) * c(0x3f26_6666)) * c(0x3f73_33db) + c(0x3d4c_c250)
        };
        let (wb, wa) = (w(b), w(a));

        // The soft clipper `x − x|x|/16` at the last sample and halfway before it.
        let sat = |x: f32| {
            let x = clamp(x, 8.0);
            x * (1.0 - x.abs() * 0.0625)
        };
        let mut o = [0f32; 4];
        for k in 0..4 {
            let h = ((self.u2[k] * 0.5625 - self.u3[k] * 0.0625) + self.u1[k] * 0.5625) - u[k] * 0.0625;
            o[k] = (if k < 2 { wb } else { wa }) * ((sat(h) + sat(self.u1[k])) * 0.5);
        }
        self.u3 = self.u2;
        self.u2 = self.u1;
        self.u1 = u;
        let dry = self.dry;
        self.dry = [l, r];
        let z = cur[S_Z];
        let out = |wet: f32, d: f32| clamp((wet - d) * z + d, 1.0);
        (out(o[0] + o[2], dry[0]), out(o[1] + o[3], dry[1]))
    }
}

// ---- Tube: NI::DSP::WarmTube (§6) -------------------------------------------------------

/// `10^x` as Apple's libm computes it (Maschine's `exp10f`).
fn exp10f(x: f32) -> f32 {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn __exp10f(x: f32) -> f32;
        }
        // SAFETY: a pure libm function.
        unsafe { __exp10f(x) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        10f32.powf(x)
    }
}

const DB_STEP: f32 = c(0x3f8f_9e5a); // 10^(1/20): dB → gain base
const TWO_PI: f32 = c(0x40c9_0f28); // 6.2831

/// WarmTube's sine and cosine polynomials: `(sin w, cos w)`.
fn sin_cos(w: f32) -> (f32, f32) {
    let x = w * w;
    let s = x
        .mul_add(c(0x32cd_4ca8), c(0x3638_b94f))
        .mul_add(x, c(0xb950_0c03))
        .mul_add(x, c(0x3c08_8885))
        .mul_add(x, c(0xbe2a_aac1))
        .mul_add(x, 1.0);
    let co = x
        .mul_add(c(0xb48b_dad9), c(0x37cf_b5a1))
        .mul_add(x, c(0xbab6_09bd))
        .mul_add(x, c(0x3d2a_aa99))
        .mul_add(x, c(0xbeff_f5e8))
        .mul_add(x, 1.0);
    (w * s, co)
}

/// `2π·f/sr` for `f` held between `sr/24576` and `sr/2.125`.
fn omega(sr: f32, f: f32) -> f32 {
    let lo = sr / 24576.0;
    let hi = sr / 2.125;
    let f = if lo > f { lo } else { f };
    let f = if f > hi { hi } else { f };
    (f * TWO_PI) / sr
}

/// Biquad coefficients `[a1, a2, b0, b1, b2]`, normalized by `a0`.
type Coefs = [f32; 5];

/// The 2nd-order lowpass (Q = 1) at `f`.
fn lowpass(sr: f32, f: f32) -> Coefs {
    let (s, co) = sin_cos(omega(sr, f));
    let al = s * 0.5;
    let inv = 1.0 / (al + 1.0);
    let omc = 1.0 - co;
    let b0 = (omc * 0.5) * inv;
    [(co * -2.0) * inv, (1.0 - al) * inv, b0, omc * inv, b0]
}

/// The 80 Hz low shelf, `db` dB.
fn shelf(sr: f32, db: f32) -> Coefs {
    let (s, co) = sin_cos(omega(sr, 80.0));
    let a = exp10f(db * c(0x3ccc_cccd));
    let (am1, ap1) = (a + -1.0, a + 1.0);
    let t = am1.mul_add(co, ap1);
    let sq = a.sqrt();
    let inv = 1.0 / (sq + sq).mul_add(s, t);
    let a1 = co.mul_add(ap1, am1) * -2.0;
    let m2 = sq * -2.0;
    let a2 = m2.mul_add(s, t);
    let u = (-am1).mul_add(co, ap1);
    let b0 = (sq + sq).mul_add(s, u) * a;
    let v = (-ap1).mul_add(co, am1);
    let b1 = (v + v) * a;
    let b2 = (a * m2.mul_add(s, u)) * inv;
    [a1 * inv, a2 * inv, b0 * inv, b1 * inv, b2]
}

/// `sign(z)·|z|^k` with WarmTube's log2/exp2 approximations.
fn tube_pow(z: f32, k: f32) -> f32 {
    let b = (if z > 0.0 - z { z } else { 0.0 - z }).to_bits();
    let m = f32::from_bits((b & 0x807f_ffff) | 0x3f80_0000) + -1.0;
    let e = ((b >> 23) & !0x100) as i32 + -127;
    let y = (m * (c(0x3fac_51ec) - m * c(0x3eb1_4749)) + e as f32) * k;
    let n = y.round_ties_even();
    let n = if n > -2_147_483_648.0 && 2_147_483_648.0 > n { n as i32 } else { i32::MIN };
    let f = y - n as f32;
    let p = 1.0 + f * (c(0x3f31_6873) + f * (c(0x3e77_ced9) + c(0x3d65_6042) * f));
    let w = p * f32::from_bits((n.wrapping_add(127) as u32) << 23);
    if z < 0.0 { -w } else { w }
}

/// The ×4 interpolator's phases (taps on `[x[n], x[n−1], x[n−2], x[n−3]]`).
const UP: [[f32; 4]; 4] = [
    [0.0, 1.0, 0.0, 0.0],
    [-0.0703125, 0.8671875, 0.2265625, -0.0234375],
    [-0.0625, 0.5625, 0.5625, -0.0625],
    [-0.0234375, 0.2265625, 0.8671875, -0.0703125],
];
/// The decimator's taps on the last four blocks of four (newest first).
const DOWN: [[f32; 4]; 4] = [
    [-0.005859375, -0.015625, -0.017578125, 0.0],
    [0.056640625, 0.140625, c(0x3e5e_0000), 0.25], // 0.216796875
    [c(0x3e5e_0000), 0.140625, 0.056640625, -0.0],
    [-0.017578125, -0.015625, -0.005859375, 0.0],
];

// Ramp lanes.
const T_CHARGE: usize = 0;
const T_TREBLE: usize = 1; // Hz
const T_BASS: usize = 2; // dB
const T_GAIN: usize = 4; // drive gain
const T_EXP: usize = 5; // drive exponent
const T_MAKEUP: usize = 6;
const T_OUT: usize = 7;

/// One channel's ×4 tube stage.
#[derive(Clone, Copy, Default)]
struct TubeChannel {
    /// Input history `[x[n], x[n−1], x[n−2], x[n−3]]`.
    hist: [f32; 4],
    /// The shaped ×4 blocks, newest first.
    blocks: [[f32; 4]; 4],
}

impl TubeChannel {
    /// One input sample (after the bias gain) to one output sample (before the makeup).
    fn process(&mut self, x: f32, a: f32, k: f32) -> f32 {
        self.hist[0] = x;
        let h = self.hist;
        let mut s = [0f32; 4];
        for (p, t) in s.iter_mut().zip(UP) {
            let z = ((t[0] * h[0] + t[2] * h[2]) + (t[1] * h[1] + t[3] * h[3])) * a;
            let w = tube_pow(z, k);
            let m = if z.abs() > 1.0 { 1.0 } else { z.abs() };
            *p = z + m * (2.0 - m) * (w - z);
        }
        let b = &self.blocks;
        let mut acc = [0f32; 4];
        for (i, v) in acc.iter_mut().enumerate() {
            *v = ((DOWN[0][i] * s[i] + b[0][i] * DOWN[1][i]) + b[1][i] * DOWN[2][i]) + b[2][i] * DOWN[3][i];
        }
        self.blocks = [s, b[0], b[1], b[2]];
        self.hist = [x, x, h[1], h[2]];
        (acc[0] + acc[2]) + (acc[1] + acc[3])
    }
}

/// Four parallel biquad lanes (direct form II).
fn biquad4(a1: [f32; 4], a2: [f32; 4], b: [[f32; 4]; 3], w1: &mut [f32; 4], w2: &mut [f32; 4], x: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    let mut w0 = [0f32; 4];
    let mut y = [0f32; 4];
    for k in 0..4 {
        w0[k] = (x[k] - a1[k] * w1[k]) - a2[k] * w2[k];
        y[k] = (b[1][k] * w1[k] + b[0][k] * w0[k]) + w2[k] * b[2][k];
    }
    (w0, y)
}

struct Tube {
    sr: f32,
    /// Drive as shaped (+0x28) and Gain in dB (+0x38).
    drive: f32,
    gain_db: f32,
    /// Bass in dB (+0x30) and Treble in Hz (+0x34).
    bass_db: f32,
    treble_hz: f32,
    /// Overload off: the input passes the two highpasses (+0x3c). EQ on (+0x3d).
    hp_on: bool,
    eq_on: bool,
    ramp: Ramp<8>,
    /// The input highpasses: integrators (+0x40), outputs (+0x50), coefficients (+0x60).
    hp_a: [f32; 4],
    hp_h: [f32; 4],
    hp_c: [f32; 4],
    ch: [TubeChannel; 2],
    /// The last outputs (+0x194) and the bias envelope (+0x19c).
    y: [f32; 2],
    env: [f32; 2],
    /// The bias envelope's and the DC blocker's one-pole coefficients (+0x190, +0x1a4).
    c_env: f32,
    c_dc: f32,
    /// 12 kHz lowpass, bass shelf, treble lowpass.
    lp12: Coefs,
    shelf: Coefs,
    treble: Coefs,
    /// EQ states: lowpass+shelf lanes (+0x1c0, +0x1d0, +0x1e0), treble+DC lanes (+0x1f0,
    /// +0x200, +0x210).
    eq1_y: [f32; 4],
    eq1_w1: [f32; 4],
    eq1_w2: [f32; 4],
    eq2_y: [f32; 4],
    eq2_w1: [f32; 4],
    eq2_w2: [f32; 4],
}

impl Tube {
    /// The constructor and `onInit` (no sample rate yet).
    fn new() -> Tube {
        let mut t = Tube {
            sr: 0.0,
            drive: c(0x3f15_1eb9),
            gain_db: 0.0,
            bass_db: 0.0,
            treble_hz: 9000.0,
            hp_on: false,
            eq_on: false,
            ramp: Ramp::new(0.0, SECS_20MS),
            hp_a: [0.0; 4],
            hp_h: [0.0; 4],
            hp_c: [0.0; 4],
            ch: [TubeChannel::default(); 2],
            y: [0.0; 2],
            env: [0.0; 2],
            c_env: 0.0,
            c_dc: 0.0,
            lp12: [0.0; 5],
            shelf: [0.0; 5],
            treble: [0.0; 5],
            eq1_y: [0.0; 4],
            eq1_w1: [0.0; 4],
            eq1_w2: [0.0; 4],
            eq2_y: [0.0; 4],
            eq2_w1: [0.0; 4],
            eq2_w2: [0.0; 4],
        };
        t.ramp.ticks = 0;
        t.ramp.inv = 0.0;
        t.ramp.tgt = [0.5, 9000.0, 0.0, 0.0, c(0x3f34_9bc8), c(0x3c3e_dfa5), c(0x3f83_db30), c(0x3f59_c7d2)];
        t.ramp.snap();
        t.reset();
        t.ramp.tgt[T_OUT] = 0.5;
        t.ramp.cur[T_OUT] = 0.5;
        t.ramp.tgt[3] = 1.0;
        t.ramp.cur[3] = 1.0;
        t
    }

    /// `onReset`: the signal state.
    fn reset(&mut self) {
        self.hp_a = [0.0; 4];
        self.hp_h = [0.0; 4];
        self.ch = [TubeChannel::default(); 2];
        self.y = [0.0; 2];
        self.env = [0.0; 2];
        self.eq1_y = [0.0; 4];
        self.eq1_w1 = [0.0; 4];
        self.eq1_w2 = [0.0; 4];
        self.eq2_y = [0.0; 4];
        self.eq2_w1 = [0.0; 4];
        self.eq2_w2 = [0.0; 4];
    }

    /// `onResume`: the on/off lane ramps to 0.
    fn resume(&mut self) {
        self.ramp.tgt[3] = 0.0;
        self.ramp.dirty = true;
    }

    /// `onSetSampleRate(sr, 128)`.
    fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr;
        self.ramp.set_sample_rate(sr, SECS_20MS);
        let w = TWO_PI / sr;
        let c12 = w * 12.0;
        let c24 = w * 24.0;
        self.c_env = if c24 > 1.0 { 1.0 } else { c24 };
        self.c_dc = if c12 > 1.0 { 1.0 } else { c12 };
        let c36 = w * 36.0;
        let c80 = w * 80.0;
        let c36 = if c36 > 1.0 { 1.0 } else { c36 };
        let c80 = if c80 > 1.0 { 1.0 } else { c80 };
        self.hp_c = [c36, c36, c80, c80];
        self.lp12 = lowpass(sr, 12000.0);
        self.treble = lowpass(sr, self.treble_hz);
        self.shelf = shelf(sr, self.bass_db);
    }

    /// `setParameter(idx, v)`.
    fn set_param(&mut self, idx: u32, v: f32) {
        match idx {
            0 => {
                let d = v.abs().mul_add(c(0xbea8_f5c3), c(0x3faa_3d71)) * v;
                self.drive = d;
                let a = DB_STEP.powf(d.mul_add(36.0, -24.0));
                let k = d * SECS_20MS;
                let m = if a.abs() > 1.0 { 1.0 } else { a.abs() };
                let q = m * (2.0 - m);
                let e = if (k as f64) < 0.001 { c(0x3a83_126f) } else { k };
                // The level the shaper gives a full-scale input: its inverse is the makeup.
                let full = (a.abs().powf(e) - a).mul_add(q, a);
                let t = &mut self.ramp.tgt;
                t[T_GAIN] = a;
                t[T_EXP] = k;
                t[T_MAKEUP] = if (full as f64) < 0.1 { 10.0 } else { 1.0 / full };
                t[T_OUT] = (d * c(0xbee1_47ae)).mul_add(d, 1.0) * DB_STEP.powf(self.gain_db);
            }
            1 => self.ramp.tgt[T_CHARGE] = v,
            2 => {
                self.hp_on = v <= 0.0;
                return;
            }
            3 => {
                self.eq_on = v <= 0.0;
                return;
            }
            4 => {
                self.bass_db = v * 12.0;
                self.ramp.tgt[T_BASS] = self.bass_db;
                self.shelf = shelf(self.sr, self.bass_db);
            }
            5 => {
                self.treble_hz = v.mul_add(5000.0, 9000.0);
                self.ramp.tgt[T_TREBLE] = self.treble_hz;
                self.treble = lowpass(self.sr, self.treble_hz);
            }
            6 => {
                self.gain_db = v;
                let d = self.drive;
                self.ramp.tgt[T_OUT] = DB_STEP.powf(v) * (d * c(0xbee1_47ae)).mul_add(d, 1.0);
            }
            _ => return,
        }
        self.ramp.dirty = true;
    }

    /// `processAudio` for one stereo sample (the control ticks come from the processor).
    fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.ramp.step();
        let cur = self.ramp.cur;
        if self.ramp.inc[T_TREBLE] != 0.0 {
            self.treble = lowpass(self.sr, cur[T_TREBLE]);
        }
        if self.ramp.inc[T_BASS] != 0.0 {
            self.shelf = shelf(self.sr, cur[T_BASS]);
        }

        // Overload off: two cascaded one-pole highpasses (36 and 80 Hz), one sample late.
        let x = if self.hp_on {
            let h = self.hp_h;
            for ((a, c), h) in self.hp_a.iter_mut().zip(self.hp_c).zip(h) {
                *a += c * h;
            }
            self.hp_h = [l - self.hp_a[0], r - self.hp_a[1], h[0] - self.hp_a[2], h[1] - self.hp_a[3]];
            [self.hp_h[2], self.hp_h[3]]
        } else {
            [l, r]
        };

        // The drive exponent, at least 0.001.
        let k = if (cur[T_EXP] as f64) < 0.001 { c(0x3a83_126f) } else { cur[T_EXP] };

        // Charge: the output level biases the input gain down.
        let mut y = [0f32; 2];
        for ch in 0..2 {
            let v = self.y[ch].abs() + c(0xbdcc_cccd);
            let v = if v < 0.0 { 0.0 } else { v };
            self.env[ch] = (v * cur[T_CHARGE] - self.env[ch]).mul_add(self.c_env, self.env[ch]);
            let e = self.env[ch];
            let e = if -2.0 > e { -2.0 } else { e };
            let e = if e > 2.0 { 2.0 } else { e };
            let g = 1.0 - e * (-0.25f32).mul_add(e.abs(), 1.0);
            let g = if c(0x3dcc_cccd) > g { c(0x3dcc_cccd) } else { g };
            y[ch] = cur[T_MAKEUP] * self.ch[ch].process(x[ch] * g, cur[T_GAIN], k);
        }
        self.y = y;

        let out = if self.eq_on {
            // 12 kHz lowpass, then (a sample later) the bass shelf.
            let lanes = |a: &Coefs, b: &Coefs, i: usize| [a[i], a[i], b[i], b[i]];
            let (l1, s1) = (&self.lp12, &self.shelf);
            let x1 = [y[0], y[1], self.eq1_y[0], self.eq1_y[1]];
            let (w0, y1) = biquad4(
                lanes(l1, s1, 0),
                lanes(l1, s1, 1),
                [lanes(l1, s1, 2), lanes(l1, s1, 3), lanes(l1, s1, 4)],
                &mut self.eq1_w1,
                &mut self.eq1_w2,
                x1,
            );
            self.eq1_w2 = self.eq1_w1;
            self.eq1_w1 = w0;
            self.eq1_y = y1;
            // Treble lowpass, then (a sample later) a 12/2π Hz DC blocker.
            let t = &self.treble;
            let cd = self.c_dc;
            let x2 = [y1[2], y1[3], self.eq2_y[0], self.eq2_y[1]];
            let a1 = [t[0], t[0], cd, cd];
            let a2 = [t[1], t[1], 1.0, 1.0];
            let b = [[t[2], t[2], 0.0, 0.0], [t[3], t[3], cd, cd], [t[4], t[4], 1.0, 1.0]];
            let (w0, y2) = biquad4(a1, a2, b, &mut self.eq2_w1, &mut self.eq2_w2, x2);
            self.eq2_w2 = [self.eq2_w1[0], self.eq2_w1[1], y2[2], y2[3]];
            self.eq2_w1 = w0;
            self.eq2_y = y2;
            [w0[2], w0[3]]
        } else {
            y
        };
        (out[0] * cur[T_OUT], out[1] * cur[T_OUT])
    }
}

// ---- The processor (§3) -------------------------------------------------------------------

/// Stored fields and defaults (`Saturator::Saturator()`): Classic, Tape, Tube. Tape's
/// Drive is stored −1..1; Tube's Charge (0..4) and Gain (dB) reach the core unnormalized.
const FIELDS: [(&str, u32); 13] = [
    ("classic_input", 0x3e80_0000),
    ("classic_drive", 0x3f00_0000),
    ("classic_contour", 0x3f40_0000),
    ("tape_input", 0x3e4c_cccd),
    ("tape_drive", 0x3eae_147b),
    ("tape_contour", 0x3f80_0000),
    ("tube_drive", 0x3e80_0000),
    ("tube_charge", 0x4000_0000),
    ("tube_overload", 0),
    ("tube_eq_bypass", 0),
    ("tube_bass", 0x3f4c_cccd),
    ("tube_treble", 0x3f4c_cccd),
    ("tube_gain", 0),
];

pub struct Saturator {
    sr: f32,
    mode: i64,
    classic: Classic,
    tape: Tape,
    tube: Tube,
    values: Option<[f32; 13]>,
    /// Samples to the Tape/Tube cores' next control tick (a grid across blocks that only
    /// runs in those modes).
    ctr: usize,
}

impl Saturator {
    pub fn new(sr: f64) -> Saturator {
        // The processor's constructor: Classic inits at 44.1 kHz, Tube without a rate. (Tape
        // takes the host's rate at once; its rate-dependent state does not depend on the
        // parameters.)
        let sr = sr as f32;
        Saturator { sr, mode: 0, classic: Classic::new(), tape: Tape::new(sr), tube: Tube::new(), values: None, ctr: 0 }
    }

    /// The bound parameter `k` of `FIELDS` with value `v` (`processXParameter`).
    fn apply(&mut self, k: usize, v: f32) {
        match k {
            0 => self.classic.set_input(v),
            1 => self.classic.set_drive(v),
            2 => self.classic.set_contour(v),
            3..=5 => self.tape.set_param(k as u32 - 3, v),
            _ => self.tube.set_param(k as u32 - 6, v),
        }
    }
}

impl Effect for Saturator {
    fn set(&mut self, params: &[(String, f64)]) {
        self.mode = param(params, "mode", 0.0) as i64;
        let mut new = FIELDS.map(|(name, def)| param(params, name, c(def) as f64) as f32);
        // Tape's Drive is sent normalized.
        new[4] = (new[4] + 1.0) * 0.5;
        match self.values {
            None => {
                // The first settings reach the processor before the host's sample rate and
                // `resume`, which re-inits Classic and resets Tape and Tube.
                for (k, v) in new.iter().enumerate().skip(3) {
                    self.apply(k, *v);
                }
                self.classic.raw = [new[0], new[1], new[2]];
                self.classic.sr = self.sr;
                self.classic.init();
                self.tape.reset();
                self.tape.resume();
                self.tube.set_sample_rate(self.sr);
                self.tube.reset();
                self.tube.resume();
            }
            Some(old) => {
                for k in 0..new.len() {
                    if new[k].to_bits() != old[k].to_bits() {
                        self.apply(k, new[k]);
                    }
                }
            }
        }
        self.values = Some(new);
    }

    /// `SaturatorProcessor::process`.
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.mode == 0 {
            for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
                (*xl, *xr) = self.classic.process(*xl, *xr);
            }
            return;
        }
        let n = l.len().min(r.len());
        let mut i = 0;
        while i < n {
            if self.ctr == 0 {
                match self.mode {
                    1 => self.tape.control_rate(),
                    2 => self.tube.ramp.tick(),
                    _ => {}
                }
                self.ctr = 128;
            }
            let len = self.ctr.min(n - i);
            match self.mode {
                1 => self.tape.process(&mut l[i..i + len], &mut r[i..i + len]),
                2 => {
                    for k in i..i + len {
                        (l[k], r[k]) = self.tube.process(l[k], r[k]);
                    }
                }
                _ => {}
            }
            self.ctr -= len;
            i += len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(kv: &[(&str, f64)]) -> Vec<(String, f64)> {
        kv.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn run(fx: &mut Saturator, x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (x.to_vec(), x.iter().map(|v| v * 0.5).collect::<Vec<_>>());
        for (a, b) in l.chunks_mut(300).zip(r.chunks_mut(300)) {
            fx.process(a, b);
        }
        (l, r)
    }

    fn sine(db: f32, n: usize) -> Vec<f32> {
        let a = 10f32.powf(db / 20.0);
        (0..n).map(|i| (i as f32 * 0.0573).sin() * a).collect()
    }

    #[test]
    fn classic_defaults_are_finite_and_bounded() {
        let mut fx = Saturator::new(48_000.0);
        fx.set(&params(&[]));
        let (l, r) = run(&mut fx, &sine(6.0, 48_000));
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() <= 1.0));
        assert!(l.iter().any(|v| v.abs() > 0.1));
    }

    #[test]
    fn every_mode_is_finite_and_silent_on_silence() {
        for mode in [0.0, 1.0, 2.0] {
            for sr in [44_100.0, 48_000.0, 96_000.0] {
                let mut fx = Saturator::new(sr);
                fx.set(&params(&[("mode", mode), ("tube_drive", 1.0), ("tube_charge", 4.0), ("tape_input", 1.0)]));
                let (l, r) = run(&mut fx, &sine(12.0, 20_000));
                assert!(l.iter().chain(&r).all(|v| v.is_finite()), "mode {mode} at {sr}");
                assert!(l[10_000..].iter().any(|v| v.abs() > 0.01), "mode {mode} at {sr}");
                // Then silence: the tails decay, and no denormal or NaN builds up.
                let (l, r) = run(&mut fx, &vec![0.0; 96_000]);
                let tail = l[90_000..].iter().chain(&r[90_000..]).fold(0f32, |m, v| m.max(v.abs()));
                assert!(tail < 1e-6, "mode {mode} at {sr}: {tail}");
            }
        }
    }

    #[test]
    fn tape_and_tube_keep_the_channels_apart() {
        for mode in [1.0, 2.0] {
            let mut fx = Saturator::new(48_000.0);
            fx.set(&params(&[("mode", mode)]));
            let mut l = sine(0.0, 9600);
            let mut r = vec![0f32; 9600];
            fx.process(&mut l, &mut r);
            assert!(l.iter().any(|v| v.abs() > 0.01), "mode {mode}");
            assert!(r.iter().all(|v| v.abs() < 1e-9), "mode {mode}");
        }
    }

    #[test]
    fn tube_drive_adds_harmonics_and_eq_bypass_differs() {
        let x = sine(-6.0, 48_000);
        let out = |kv: &[(&str, f64)]| {
            let mut fx = Saturator::new(48_000.0);
            fx.set(&params(kv));
            run(&mut fx, &x).0
        };
        let soft = out(&[("mode", 2.0), ("tube_drive", 0.0)]);
        let hard = out(&[("mode", 2.0), ("tube_drive", 1.0)]);
        let flat = out(&[("mode", 2.0), ("tube_drive", 1.0), ("tube_eq_bypass", 1.0)]);
        let diff = |a: &[f32], b: &[f32]| a[24_000..].iter().zip(&b[24_000..]).fold(0f32, |m, (p, q)| m.max((p - q).abs()));
        assert!(diff(&soft, &hard) > 1e-2);
        assert!(diff(&hard, &flat) > 1e-3);
    }

    #[test]
    fn mode_switch_keeps_each_cores_state() {
        let x = sine(-6.0, 9600);
        let p = |mode: f64| params(&[("mode", mode), ("classic_drive", 0.8), ("tube_drive", 0.7)]);
        let mut fx = Saturator::new(48_000.0);
        fx.set(&p(0.0));
        run(&mut fx, &x);
        fx.set(&p(2.0));
        let tube = run(&mut fx, &x).0;
        assert!(tube.iter().all(|v| v.is_finite()));
        // Back to Classic: it carries on as if the Tube block had not happened.
        let mut alone = Saturator::new(48_000.0);
        alone.set(&p(0.0));
        run(&mut alone, &x);
        let a = run(&mut alone, &x);
        fx.set(&p(0.0));
        let b = run(&mut fx, &x);
        assert_eq!(a, b);
    }

    #[test]
    fn parameter_changes_ramp() {
        let x = sine(-20.0, 4800);
        let mut fx = Saturator::new(48_000.0);
        fx.set(&params(&[("mode", 2.0), ("tube_drive", 0.0), ("tube_gain", 0.0)]));
        run(&mut fx, &x);
        fx.set(&params(&[("mode", 2.0), ("tube_drive", 0.0), ("tube_gain", -12.0)]));
        let y = run(&mut fx, &x).0;
        let peak = |s: &[f32]| s.iter().fold(0f32, |m, v| m.max(v.abs()));
        // The Gain glides over ≈ 20 ms (8 ticks of 128 at 48 kHz) to −12 dB.
        assert!(peak(&y[..128]) > 1.5 * peak(&y[2000..]), "{} {}", peak(&y[..128]), peak(&y[2000..]));
    }

    #[test]
    fn tube_pow_approximates_a_power() {
        for z in [0.01f32, 0.3, 1.0, 2.5, -0.7] {
            for k in [0.001f32, 0.01, 0.02] {
                let exact = z.signum() * z.abs().powf(k);
                assert!((tube_pow(z, k) - exact).abs() < 2e-3 * exact.abs(), "{z} {k}");
            }
        }
    }

    #[test]
    fn classic_gain_table_rows() {
        // Row 0 is the identity curve; every row ends at 1.
        for i in 0..128 {
            assert!((c(gain_table::GAIN[i]) - i as f32 / 127.0).abs() < 1e-6);
        }
        for row in 0..51 {
            assert_eq!(c(gain_table::GAIN[row * 128 + 127]), 1.0);
        }
    }
}
