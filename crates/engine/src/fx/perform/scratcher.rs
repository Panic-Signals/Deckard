//! Scratcher (docs/fx-perform-scratcher.md): Maschine's turntable Perform FX. Touching the
//! strip catches the record: what was just heard plays on while a brake slows it to a
//! stop (slower as the strip goes up), then the needle follows the strip (scratching),
//! never past real time. A "swirl" runs over it: a delay (its time scaled by the strip)
//! whose feedback goes through a 15 kHz stage, a Hilbert pair and a frequency shifter, a
//! sine saturator and a 30 Hz stage.
//!
//! Sample for sample as Maschine 3 computes it: `f32` unless the spec says `f64`, fused
//! multiply-adds where it has them (`mul_add`), the system's `sinf`, `tanh`, `exp10` and
//! `sincos`. The strip arrives as the raw `touchstrip_pos` and `touchstrip_on_off`
//! parameters (Maschine smooths them itself); `Ctx::pos` is not used.

use super::{Ctx, Mode, Params};

const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}

const fn d(bits: u64) -> f64 {
    f64::from_bits(bits)
}

/// The tan series' coefficients (§3.1): ~1/3 and 2/15.
const T3: f64 = d(0x3fd5_5555_5555_554f);
const T5: f64 = d(0x3fc1_1111_1111_1111);
/// 2π·10⁴: the primes' cutoff argument.
const PRIME_W: f64 = d(0x40ee_adfb_4c5d_390c);
const TAU: f64 = d(0x4019_21fb_5444_2d18);
const PI: f64 = d(0x4009_21fb_5444_2d18);
/// `f64(0.001f)`.
const MS: f64 = d(0x3f50_624d_e000_0000);
/// 0.05.
const F05: f64 = d(0x3fa9_9999_9999_999a);

fn exp10(x: f64) -> f64 {
    #[cfg(target_vendor = "apple")]
    {
        unsafe extern "C" {
            fn __exp10(x: f64) -> f64;
        }
        // SAFETY: a pure libm function.
        unsafe { __exp10(x) }
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        10f64.powf(x)
    }
}

/// (sin, cos).
fn sincos(x: f64) -> (f64, f64) {
    #[cfg(target_vendor = "apple")]
    {
        #[repr(C)]
        struct SinCos {
            sin: f64,
            cos: f64,
        }
        unsafe extern "C" {
            fn __sincos_stret(x: f64) -> SinCos;
        }
        // SAFETY: a pure libm function.
        let r = unsafe { __sincos_stret(x) };
        (r.sin, r.cos)
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        (x.sin(), x.cos())
    }
}

/// ARM's `FRECPE` (8-bit reciprocal estimate) for normal, finite `x`.
#[cfg(any(not(target_arch = "aarch64"), test))]
fn frecpe(x: f32) -> f32 {
    let b = x.to_bits();
    let exp = (b >> 23) & 0xff;
    let a = (256 + ((b >> 15) & 0xff)) * 2 + 1; // '1':fraction<22:15>, rounded to odd
    let est = ((1u32 << 19) / a).div_ceil(2); // 256..511
    f32::from_bits((b & 0x8000_0000) | ((253 - exp) << 23) | ((est & 0xff) << 15))
}

/// `1/x` as NEON `frecpe` and two `frecps` steps.
fn recip(x: f32) -> f32 {
    #[cfg(target_arch = "aarch64")]
    {
        use std::arch::aarch64::*;
        // SAFETY: NEON is always present on aarch64.
        unsafe {
            let v = vdup_n_f32(x);
            let r = vrecpe_f32(v);
            let r = vmul_f32(r, vrecps_f32(v, r));
            vget_lane_f32::<0>(vmul_f32(vrecps_f32(v, r), r))
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let r = frecpe(x);
        let r = r * (-x).mul_add(r, 2.0);
        (-x).mul_add(r, 2.0) * r
    }
}

/// Eigen 3.4's vectorised `sin` (`psin_float`) for |x| well under 117435.
fn psin(x: f32) -> f32 {
    let ax = x.abs();
    let yr = ax * c(0x3f22_f983) + c(0x4b40_0000);
    let q = yr.to_bits();
    let y = yr + c(0xcb40_0000);
    let mut r = c(0xbfc9_0fd8).mul_add(y, ax);
    r = c(0xb4a8_885a).mul_add(y, r);
    r = c(0xa7c2_34c4).mul_add(y, r);
    let sign = ((q << 30) ^ x.to_bits()) & 0x8000_0000;
    let x2 = r * r;
    let v = if q & 1 == 0 {
        let p = x2.mul_add(c(0xb94d_70ca).mul_add(x2, c(0x3c08_85d3)), c(0xbe2a_aaa8));
        r.mul_add(x2 * p, r)
    } else {
        let p = x2.mul_add(c(0x37cc_730b).mul_add(x2, c(0xbab6_036e)), c(0x3d2a_aa9e));
        x2.mul_add(x2.mul_add(p, -0.5), 1.0)
    };
    f32::from_bits(v.to_bits() ^ sign)
}

/// The first column of `M⁻¹`, `M[j][k] = (−1)ᵏ(k+1)^(2j+1)` (§5.3), as Eigen computes it in
/// `f32`: partial-pivot LU (dividing by the pivot, the update unfused), then the unit
/// lower and the upper solves of the permuted unit vector (fused updates, the upper by
/// the pivot's reciprocal).
fn saturator_column() -> [f32; 5] {
    let mut m = [[0.0f32; 5]; 5];
    for (j, row) in m.iter_mut().enumerate() {
        for (k, v) in row.iter_mut().enumerate() {
            let p = ((k + 1) as f32).powi(2 * j as i32 + 1);
            *v = if k % 2 == 0 { p } else { -p };
        }
    }
    let mut x = [1.0f32, 0.0, 0.0, 0.0, 0.0];
    for k in 0..5 {
        let mut best = k;
        for r in k + 1..5 {
            if m[r][k].abs() > m[best][k].abs() {
                best = r;
            }
        }
        if m[best][k] != 0.0 {
            m.swap(k, best);
            x.swap(k, best);
            for r in k + 1..5 {
                m[r][k] /= m[k][k];
            }
        }
        for r in k + 1..5 {
            for col in k + 1..5 {
                m[r][col] -= m[r][k] * m[k][col];
            }
        }
    }
    for i in 0..5 {
        let b = -x[i];
        for r in i + 1..5 {
            x[r] = b.mul_add(m[r][i], x[r]);
        }
    }
    for i in (0..5).rev() {
        let inv = 1.0 / m[i][i];
        let b = x[i];
        x[i] = inv * b;
        let nb = -(inv * b);
        for r in 0..i {
            x[r] = nb.mul_add(m[r][i], x[r]);
        }
    }
    x
}

/// `v` clamped to 0..1 as Maschine does (NaN passes).
fn unit(v: f32) -> f32 {
    let w = if v > 1.0 { 1.0 } else { v };
    if v < 0.0 { 0.0 } else { w }
}

/// A linear ramp to its target (§1.1).
#[derive(Debug, Clone, Copy, Default)]
struct Glide {
    cur: f32,
    target: f32,
    step: f32,
    count: i32,
    steps: i32,
    dur: f32,
    pending: bool,
}

impl Glide {
    fn to(&mut self, t: f32) {
        self.target = t;
        if self.cur != t {
            self.step = (t - self.cur) / self.dur;
            self.count = self.steps;
            self.pending = true;
        }
    }

    fn snap(&mut self) {
        if self.pending {
            self.cur = self.target;
            self.step = 0.0;
            self.count = 0;
            self.pending = false;
        }
    }

    fn tick(&mut self) {
        if (self.target - self.cur).abs() > 0.0 {
            let n = self.count;
            self.count -= 1;
            if n >= 2 {
                self.cur += self.step;
                return;
            }
        }
        self.snap();
    }

    fn timing(&mut self, dur: f32) {
        self.dur = dur;
        self.steps = dur.round() as i32;
        self.count = self.steps;
    }
}

/// The `f64` two-pole low-pass (§3.1).
#[derive(Debug, Clone, Copy, Default)]
struct Svf {
    a1: f64,
    a2: f64,
    a3: f64,
    ic1: f64,
    ic2: f64,
}

impl Svf {
    fn coefs(w: f64) -> (f64, f64, f64) {
        let w2 = w * w;
        let w3 = w * w2;
        let w5 = w2 * w3;
        let g = w5.mul_add(T5, w3.mul_add(T3, w));
        let a1 = 1.0 / g.mul_add(g + 2.0, 1.0);
        let a2 = g * a1;
        (a1, a2, g * a2)
    }

    fn tune(&mut self, w: f64) {
        (self.a1, self.a2, self.a3) = Svf::coefs(w);
    }

    fn step(&mut self, x: f64) -> f64 {
        let v3 = x - self.ic2;
        let v1 = self.a1.mul_add(self.ic1, v3 * self.a2);
        let v2 = self.ic2 + self.a2.mul_add(self.ic1, v3 * self.a3);
        self.ic1 = v1.mul_add(2.0, -self.ic1);
        self.ic2 = v2.mul_add(2.0, -self.ic2);
        v2
    }

    /// From zero, `n` steps towards `x` with the cutoff `w` (the filter's own kept).
    fn prime(&mut self, n: u64, w: f64, x: f64) {
        let mut p = Svf { ic1: 0.0, ic2: 0.0, ..*self };
        p.tune(w);
        for _ in 0..n {
            p.step(x);
        }
        (self.ic1, self.ic2) = (p.ic1, p.ic2);
    }
}

/// The swirl's one-pole stages (§5.1).
#[derive(Debug, Clone, Copy, Default)]
struct OnePole {
    g: f32,
    gg: f32,
    s: f32,
}

impl OnePole {
    fn tune(&mut self, isr: f32, k: f32) {
        let x = isr * k;
        let x2 = x * x;
        let x3 = x * x2;
        let x5 = x2 * x3;
        self.g = x5.mul_add(c(0x3e08_8889), x3.mul_add(c(0x3eaa_aaab), x));
        self.gg = 1.0 / (self.g + 1.0);
    }

    /// (u, o).
    fn tick(&mut self, x: f32) -> (f32, f32) {
        let u = x + self.gg * (-self.g).mul_add(x, self.s * -2.0);
        let o = self.s.mul_add(2.0, self.g * u);
        self.s = self.g.mul_add(u, self.s);
        (u, o)
    }
}

/// One all-pass stage of the Hilbert pair: four lanes, an 8-slot ring (§5.2).
#[derive(Debug, Clone, Copy, Default)]
struct AllPass {
    ring: [[f32; 4]; 8],
    at: usize,
    lag: usize,
    k: [f32; 4],
}

impl AllPass {
    fn tick(&mut self, x: [f32; 4]) -> [f32; 4] {
        let old = self.ring[(self.at + 9 - self.lag) & 7];
        let mut y = [0.0; 4];
        let mut keep = [0.0; 4];
        for i in 0..4 {
            let t = self.k[i] * (x[i] - old[i]);
            keep[i] = x[i] + t;
            y[i] = old[i] + t;
        }
        self.at = (self.at + 1) & 7;
        self.ring[self.at] = keep;
        y
    }
}

/// Denominators and numerators of the first six stages' coefficients (A, B lanes).
const HILBERT: [([u32; 2], [u32; 2]); 6] = [
    ([0x3f66_b89e, 0x3f52_5b55], [0x3cee_cf60, 0x3e96_b02e]),
    ([0x3f44_97b1, 0x3f3c_49d4], [0x3ef2_7320, 0x3f14_e7c6]),
    ([0x3f37_78d4, 0x3f34_7356], [0x3f24_f5c6, 0x3f2f_0815]),
    ([0x3f8c_a3b1, 0x3f96_d256], [0xbf21_1015, 0xbf64_f1b1]),
    ([0x3f9d_b428, 0x3fa1_db16], [0xbf89_6995, 0xbf97_40b0]),
    ([0x3fa4_4396, 0x3fa5_c655], [0xbf9f_47b0, 0xbfa4_50d8]),
];

/// A buffer of mono pairs, written at `at` (the next slot).
#[derive(Debug, Clone, Default)]
struct Ring {
    buf: [Vec<f32>; 2],
    at: usize,
}

impl Ring {
    fn size(&mut self, n: usize) {
        for b in &mut self.buf {
            b.resize(n.max(4), 0.0);
        }
        self.at %= n.max(4);
    }

    fn len(&self) -> usize {
        self.buf[0].len()
    }

    fn clear(&mut self) {
        for b in &mut self.buf {
            b.fill(0.0);
        }
        self.at = 0;
    }

    fn push(&mut self, l: f32, r: f32) {
        self.buf[0][self.at] = l;
        self.buf[1][self.at] = r;
        self.at = (self.at + 1) % self.len();
    }

    /// `back` samples before the write position (1: the last written).
    fn get(&self, ch: usize, back: usize) -> f32 {
        let n = self.len();
        self.buf[ch][(n + self.at - back % n) % n]
    }

    /// The 4-point cubic at offset `i` (from the write position) and fraction `t`.
    fn cubic(&self, ch: usize, i: i64, t: f64) -> f32 {
        let n = self.len() as i64;
        let base = n + self.at as i64 + i;
        let y = |k: i64| self.buf[ch][(base + k).rem_euclid(n) as usize];
        let (y0, y1, y2, y3) = (y(-1), y(0), y(1), y(2));
        let c3 = y3.mul_add(0.5, y2.mul_add(-1.5, y0.mul_add(-0.5, y1 * 1.5)));
        let c2 = y3.mul_add(-0.5, y2.mul_add(2.0, y1.mul_add(-2.5, y0)));
        let c1 = y0.mul_add(-0.5, y2 * 0.5);
        let t2 = t * t;
        let t3 = t * t2;
        (f64::from(c1).mul_add(t, f64::from(c3).mul_add(t3, t2 * f64::from(c2))) + f64::from(y1)) as f32
    }
}

/// `fcvtzs(r)`, one less when `low`: the read offset Maschine takes for `r`.
fn offset(r: f64, low: bool) -> i64 {
    i64::from(r as i32 - i32::from(low))
}

pub struct Scratcher {
    sr: f32,
    isr: f64,
    /// Raw parameters as last given (the core sees changes only).
    given: [Option<f32>; 6],
    active: bool,
    fresh: bool,
    param: [f32; 6],
    pos: Glide,
    on: Glide,
    scr: Glide,
    swl: Glide,
    fb: Glide,
    lo: Glide,
    hi: Glide,
    hz: Glide,
    touched: bool,
    dirty_on: bool,
    dirty_pos: bool,
    suspended: bool,
    prev_on: f32,
    brake_reset: bool,
    braking: bool,
    n_since: u64,
    n_rec: u64,
    rec_len: u64,
    a: Ring,
    b: Ring,
    c: Ring,
    brake: Svf,
    needle: Svf,
    time: Svf,
    c8: f64,
    c0: f64,
    travel: f64,
    prev_pos: f64,
    delta: f64,
    f15: [OnePole; 2],
    f30: [OnePole; 2],
    hilbert: [AllPass; 7],
    re: f64,
    im: f64,
    sat_a: [f32; 5],
    sat_c: [f32; 5],
    sat_g: f32,
}

impl Scratcher {
    pub fn new(sr: f32) -> Scratcher {
        let pi = c(0x4049_0fdb);
        let sat_a = [pi, 2.0 * pi, 3.0 * pi, 4.0 * pi, c(0x417b_53d2)];
        let sat_c = saturator_column().map(|v| v * c(0x3ea2_f983));
        let (k, a) = (sat_c, sat_a);
        let sum = (k[0] * a[0] + k[2] * a[2]) + (k[1] * a[1] + k[3] * a[3]) + k[4] * a[4];
        let mut s = Scratcher {
            sr: 1.0,
            isr: 1.0,
            given: [None; 6],
            active: false,
            fresh: true,
            param: [0.0; 6],
            pos: Glide::default(),
            on: Glide::default(),
            scr: Glide::default(),
            swl: Glide::default(),
            fb: Glide::default(),
            lo: Glide::default(),
            hi: Glide::default(),
            hz: Glide::default(),
            touched: false,
            dirty_on: true,
            dirty_pos: true,
            suspended: true,
            prev_on: 0.0,
            brake_reset: false,
            braking: true,
            n_since: 0,
            n_rec: 0,
            rec_len: 0,
            a: Ring::default(),
            b: Ring::default(),
            c: Ring::default(),
            brake: Svf::default(),
            needle: Svf::default(),
            time: Svf::default(),
            c8: 0.5,
            c0: 0.0,
            travel: 0.0,
            prev_pos: 1.0,
            delta: 0.0,
            f15: [OnePole::default(); 2],
            f30: [OnePole::default(); 2],
            hilbert: [AllPass::default(); 7],
            re: 0.0,
            im: 0.0,
            sat_a,
            sat_c,
            sat_g: 1.0 / (sum + 1.0),
        };
        // onInit, then the processor's 44.1 kHz setup and the host's rate.
        s.param[2] = 1.0;
        s.scr.to(0.8);
        s.swl.to(0.7);
        s.rate(44_100.0);
        if sr != 44_100.0 {
            s.suspend();
            s.rate(sr);
        }
        s
    }

    /// `onSetSampleRate`.
    fn rate(&mut self, sr: f32) {
        if !self.suspended || sr < 44_100.0 {
            return;
        }
        self.sr = sr;
        let srd = f64::from(sr);
        self.isr = 1.0 / srd;
        let ms = sr * c(0x3a83_126f);
        self.pos.timing(ms * 5.0);
        for g in [&mut self.on, &mut self.scr, &mut self.swl, &mut self.fb, &mut self.lo, &mut self.hi, &mut self.hz] {
            g.timing(ms * 20.0);
        }
        self.rec_len = (srd * 0.8) as u64;
        let n = self.rec_len as i32 as usize;
        self.a.size(n);
        self.c.size(n);
        self.b.size(srd.mul_add(F05, 10.0) as u64 as usize);
        // The shifter.
        let lag = (((sr + 1.0) / 44_100.0) as i32).min(8) as usize;
        for (i, st) in self.hilbert.iter_mut().enumerate() {
            st.k = match HILBERT.get(i) {
                Some(&([da, db], [na, nb])) => {
                    let (a, b) = (recip(c(da)) * c(na), recip(c(db)) * c(nb));
                    [a, b, a, b]
                }
                None => [recip(1.0) * c(0xbe99_999a); 4],
            };
            st.lag = 1;
            if lag != 1 {
                st.lag = lag;
                st.ring = [[0.0; 4]; 8];
                st.at = 0;
            }
        }
        let isr = 1.0 / sr;
        for f in &mut self.f30 {
            f.tune(isr, c(0x42bc_7edd));
            f.s = 0.0;
        }
        for f in &mut self.f15 {
            f.tune(isr, c(0x4738_13e4));
            f.s = 0.0;
        }
        let w = self.isr * d(0x4032_d97c_7f33_21d2);
        self.needle.tune(w);
        self.time.tune(w);
    }

    fn suspend(&mut self) {
        self.suspended = true;
        self.pos.snap();
    }

    /// `onReset`, then `onResume`.
    fn reset(&mut self) {
        self.pos.snap();
        for r in [&mut self.a, &mut self.b, &mut self.c] {
            r.clear();
        }
        (self.re, self.im) = (1.0, 0.0);
        for st in &mut self.hilbert {
            st.ring = [[0.0; 4]; 8];
            st.at = 0;
        }
        for f in self.f15.iter_mut().chain(self.f30.iter_mut()) {
            f.s = 0.0;
        }
        self.dirty_on = true;
        self.dirty_pos = true;
        self.brake_reset = true;
        self.prev_on = 0.0;
        (self.on.cur, self.on.target, self.on.step, self.on.count) = (0.0, 0.0, 0.0, 0);
        for g in [&mut self.scr, &mut self.swl, &mut self.fb, &mut self.lo, &mut self.hi, &mut self.hz] {
            g.snap();
        }
        (self.n_since, self.n_rec) = (0, 0);
        self.c0 = 1.0;
        self.prev_pos = 0.0;
        self.travel = 0.0;
        self.time.prime(20, self.isr * PRIME_W, MS);
        self.strip_on();
        self.strip_pos();
        self.reset_brake();
        self.suspended = false;
    }

    fn reset_brake(&mut self) {
        let n = (self.sr * c(0x3a03_126f)) as u64;
        let w = self.isr * PRIME_W;
        (self.brake.ic1, self.brake.ic2) = (0.0, 0.0);
        (self.needle.ic1, self.needle.ic2) = (0.0, 0.0);
        if n != 0 {
            self.brake.prime(n, w, 1.0);
            self.needle.prime(n, w, self.c8);
        }
    }

    /// The position, if dirty (§4.1 step 1).
    fn strip_pos(&mut self) {
        if !self.dirty_pos {
            return;
        }
        let x = self.param[0];
        if !self.suspended && self.prev_on != 0.0 {
            if !self.touched {
                return;
            }
            self.pos.to(x);
        } else {
            (self.pos.cur, self.pos.target, self.pos.step, self.pos.count) = (x, x, 0.0, 0);
        }
        self.dirty_pos = false;
    }

    /// On/off, if dirty and the last change is done (§4.1 step 2).
    fn strip_on(&mut self) {
        if !self.dirty_on || self.on.pending {
            return;
        }
        let x = self.param[1];
        if x > 0.5 {
            if !self.touched {
                (self.n_since, self.n_rec) = (0, 0);
                let (la, lb) = (self.a.len(), self.b.len());
                for k in (1..=lb).rev() {
                    for ch in 0..2 {
                        self.a.buf[ch][(la + self.a.at - k % la) % la] = self.b.get(ch, k);
                    }
                }
            }
            self.on.to(1.0);
        } else if self.touched {
            self.on.to(0.0);
            if self.pos.pending {
                self.pos.target = self.pos.cur;
                self.pos.step = 0.0;
                self.pos.count = 0;
                self.dirty_pos = true;
            }
        }
        self.touched = x > 0.5;
        self.dirty_on = false;
    }

    /// `setParameter`.
    fn set_param(&mut self, i: usize, raw: f32) {
        self.param[i] = raw;
        let xd = f64::from(unit(raw));
        match i {
            0 => self.dirty_pos = true,
            1 => self.dirty_on = true,
            2 => {
                self.scr.to(xd.mul_add(f64::from(-0.2f32), 1.0) as f32);
                self.swl.to(xd.mul_add(f64::from(0.7f32), 0.0) as f32);
            }
            3 => {
                self.hi.to(xd.mul_add(38.0, 12.0) as f32);
                self.lo.to(xd.mul_add(30.0, -10.0) as f32);
            }
            4 => {
                let tenth = c(0x3dcc_cccd);
                let u = unit((raw + c(0xbdcc_cccd)) / c(0x3f66_6666));
                let v = unit(raw / tenth);
                let f = if raw >= tenth || raw.is_nan() {
                    f64::from(u).mul_add(f64::from(0.6f32), f64::from(0.4f32))
                } else {
                    f64::from(v).mul_add(f64::from(0.4f32), 0.0)
                };
                self.fb.to(f as f32 * c(0x3f7d_70a4));
            }
            5 => {
                let s = xd.mul_add(2.0, -1.0) as f32;
                self.hz.to(s * if s > 0.0 { 50.0 } else { 100.0 });
            }
            _ => {}
        }
    }

    fn saturate(&self, y: f32) -> f32 {
        let x = self.sat_g * y;
        let x = if x < -1.0 { -1.0 } else { x };
        let x = if x > 1.0 { 1.0 } else { x };
        let (k, a) = (&self.sat_c, &self.sat_a);
        let s = |i: usize| k[i] * psin(x * a[i]);
        let sum = (s(0) + s(2)) + (s(1) + s(3));
        x + (sum + (x * a[4]).sin() * k[4])
    }

    /// The needle (§4.3): adds the record to `mix`.
    fn needle(&mut self, on: f32, mix: &mut [f32; 2]) {
        let p_raw = self.pos.cur;
        let p = f64::from(p_raw);
        let p = if p > 1.0 { 1.0 } else { p };
        let p = if p_raw < 0.0 { 0.0 } else { p };
        let s = p.mul_add(d(0x3fdc_cccc_cccc_cccd), -F05);
        let c8 = if s < F05 { F05 } else { s };
        self.c8 = c8;
        self.brake.tune(d(0x3fd4_5a1c_ac08_3127) / c8 * PI * self.isr);
        let (x, jump);
        if self.brake_reset {
            self.reset_brake();
            self.travel = 0.0;
            self.c0 = c8;
            (self.prev_pos, self.delta) = (-1.0, 1.0);
            self.braking = true;
            self.brake_reset = false;
            (x, jump) = (c8, false);
        } else if !self.braking {
            (x, jump) = (s, false);
        } else if self.delta >= MS || self.n_since < 3 {
            (x, jump) = (c8, false);
        } else {
            self.braking = false;
            (x, jump) = (s, s != c8);
        }
        let v = self.brake.step(0.0);
        self.travel = self.isr.mul_add(v, self.travel);
        let f = self.travel / self.c0;
        let f = if f > 1.0 { 1.0 } else { f };
        let f = if f < 0.0 { 0.0 } else { f };
        if jump {
            let n = (self.sr * c(0x3a83_126f)) as u64;
            (self.needle.ic1, self.needle.ic2) = (0.0, 0.0);
            if n != 0 {
                self.needle.prime(n, self.isr * PRIME_W, x);
            }
            (self.brake.ic1, self.brake.ic2) = (0.0, 0.0);
        }
        let mut at = f * self.needle.step(x);
        if at > self.travel {
            let lim = self.isr * self.n_since as f64 - self.travel;
            let lim = if lim > 1e10 { 1e10 } else { lim };
            let tiny = d(0x3ddb_7cdf_d9d7_bdbb);
            let lim = if lim < tiny { tiny } else { lim };
            at = ((at - self.travel) / lim).tanh().mul_add(lim, self.travel);
        }
        at *= f64::from(self.sr);
        self.delta = at - self.prev_pos;
        self.prev_pos = at;
        if jump {
            self.delta = 0.0;
        }
        let rd = at - (self.n_rec + 4) as f64;
        let g = self.delta.abs() / d(0x3fc9_9999_9999_999a);
        let gain = (if g > 1.0 { 1.0 } else { g }) as f32;
        let i = offset(rd, rd <= 0.0);
        let t = rd - i as f64;
        for (ch, m) in mix.iter_mut().enumerate() {
            *m = (on * self.a.cubic(ch, i, t)).mul_add(gain, *m);
        }
        self.n_since += 1;
    }
}

impl Mode for Scratcher {
    fn set(&mut self, p: &Params) {
        let active = p.i("mode") == 7;
        if active && !self.active {
            self.fresh = true;
        }
        self.active = active;
        let raw = [
            p.f("touchstrip_pos"),
            if p.f("touchstrip_on_off") >= 0.5 { 1.0 } else { 0.0 },
            p.f("scratcher_wet_level"),
            p.f("scratcher_delay_time"),
            p.f("scratcher_feedback"),
            p.f("scratcher_freq_shift"),
        ];
        for (i, v) in raw.into_iter().enumerate() {
            if self.given[i] != Some(v) {
                self.given[i] = Some(v);
                self.set_param(i, v);
            }
        }
    }

    fn frame(&mut self, l: f32, r: f32, cx: &Ctx) -> (f32, f32) {
        if self.fresh {
            // Switched to (or the first run): onReset, onResume.
            self.fresh = false;
            self.reset();
        }
        // Leaving (another mode crossfading in): as if let go.
        let on_param = if cx.engaged { self.given[1].unwrap_or(0.0) } else { 0.0 };
        if self.param[1] != on_param {
            self.set_param(1, on_param);
        }
        self.strip_pos();
        self.strip_on();
        for g in [&mut self.pos, &mut self.on, &mut self.scr, &mut self.swl, &mut self.fb, &mut self.lo, &mut self.hi, &mut self.hz] {
            g.tick();
        }
        let on = self.on.cur;
        if on == 0.0 && self.prev_on > 0.0 {
            self.pos.snap();
        }
        self.prev_on = on;
        let srd = f64::from(self.sr);
        // Record and dry.
        let rec = (on == 0.0 && !self.touched) || srd.mul_add(d(0xbfac_28f5_c28f_5c2a), self.rec_len as f64) > self.n_rec as f64;
        if rec {
            self.n_rec += 1;
            self.a.push(l, r);
        }
        self.b.push(l, r);
        let dry = [self.b.get(0, 4), self.b.get(1, 4)];
        let off = 1.0 - on;
        let mut mix = [off * dry[0], off * dry[1]];
        let p_raw = self.pos.cur;
        if on > 0.0 || self.touched {
            self.needle(on, &mut mix);
        } else {
            self.brake_reset = true;
        }
        // The swirl.
        let (lo, hi, hz) = (self.lo.cur, self.hi.cur, self.hz.cur);
        let (sn, cs) = sincos(f64::from(hz) * TAU / srd);
        let db = f64::from(unit(p_raw)).mul_add(f64::from(hi - lo), f64::from(lo)) as f32;
        let secs = (exp10(f64::from(db) * F05) * d(0x3f50_624d_d2f1_a9fc)) as f32;
        let dly = self.sr * self.time.step(f64::from(secs)) as f32;
        let rd = -f64::from(dly);
        let i = offset(rd, dly >= 0.0 || dly.is_nan());
        let t = rd - i as f64;
        let wet = [self.c.cubic(0, i, t), self.c.cubic(1, i, t)];
        let fb = self.fb.cur;
        let o = [0, 1].map(|ch| self.f15[ch].tick(on * wet[ch].mul_add(fb, mix[ch])).1);
        let mut v = [o[0], o[0], o[1], o[1]];
        let mut a6 = v;
        for (k, st) in self.hilbert.iter_mut().enumerate() {
            v = st.tick(v);
            if k == 5 {
                a6 = v;
            }
        }
        let re = cs.mul_add(self.re, -(sn * self.im)) + 0.0;
        let im = sn.mul_add(self.re, cs * self.im);
        (self.re, self.im) = (re, im);
        let (re, im) = (re as f32, im as f32);
        let y = [im * a6[0] + re * v[1], im * a6[2] + re * v[3]];
        let u = [0, 1].map(|ch| {
            let s = self.saturate(y[ch]);
            self.f30[ch].tick(s).0
        });
        self.c.push(u[0], u[1]);
        let (scr, swl) = (self.scr.cur, self.swl.cur);
        let out = |ch: usize| off.mul_add(dry[ch], on * (mix[ch] * scr + swl * wet[ch]));
        (out(0), out(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44_100.0;

    fn params(extra: &[(&str, f64)]) -> Vec<(String, f64)> {
        let mut p: Vec<(String, f64)> = deckard_model::PERFORM_FX.1.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        for (k, v) in [("mode", 7.0), ("touchstrip_on_off", 1.0)].iter().chain(extra) {
            match p.iter_mut().find(|(n, _)| n == k) {
                Some(x) => x.1 = *v,
                None => p.push((k.to_string(), *v)),
            }
        }
        p
    }

    fn run(sr: f32, p: &[(String, f64)], input: &[[f32; 2]]) -> Vec<[f32; 2]> {
        run_faded(sr, p, input, false)
    }

    /// With `fade`, the input fades in as `PerformanceFXProcessor`'s switch to the mode
    /// at load does it (20 ms, the processor's ramp; its output crossfade from the old
    /// mode isn't modelled, so compare after the fade).
    fn run_faded(sr: f32, p: &[(String, f64)], input: &[[f32; 2]], fade: bool) -> Vec<[f32; 2]> {
        let mut s = Scratcher::new(sr);
        s.set(&Params(p));
        let cx = Ctx { sr, pos: 0.5, engaged: true, quarter: sr / 2.0, song: None };
        let dur = sr * 0.001 * 20.0;
        let mut g = Glide { dur, steps: dur as i32, ..Glide::default() };
        g.to(if fade { 1.0 } else { 0.0 });
        if !fade {
            g.cur = 1.0;
            g.target = 1.0;
            g.pending = false;
        }
        input
            .iter()
            .map(|&[l, r]| {
                g.tick();
                s.frame(g.cur * l, g.cur * r, &cx).into()
            })
            .collect()
    }

    /// The samples a null test skips: the load crossfade.
    fn skip(sr: f32) -> usize {
        (sr * 0.001 * 20.0) as usize
    }

    fn noise(n: usize) -> Vec<[f32; 2]> {
        let mut seed = 7u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1 << 24) as f32 - 0.5
        };
        (0..n).map(|_| [next() * 0.5, next() * 0.5]).collect()
    }

    fn rms(x: impl Iterator<Item = f32>) -> f32 {
        let (s, n) = x.fold((0.0f64, 0usize), |(s, n), v| (s + f64::from(v) * f64::from(v), n + 1));
        (s / n.max(1) as f64).sqrt() as f32
    }

    #[test]
    fn frecpe_matches_the_instruction() {
        for x in [0.901_254_5f32, 1.0, 1.295_115, 0.704_884_9, 3.0, 1e-3] {
            let e = frecpe(x);
            #[cfg(target_arch = "aarch64")]
            {
                use std::arch::aarch64::*;
                // SAFETY: NEON.
                let n = unsafe { vget_lane_f32::<0>(vrecpe_f32(vdup_n_f32(x))) };
                assert_eq!(e.to_bits(), n.to_bits(), "{x}");
            }
            assert!((e * x - 1.0).abs() < 0.01, "{x}: {e}");
        }
    }

    #[test]
    fn psin_is_a_sine() {
        for i in -200..=200 {
            let x = i as f32 * 0.0785;
            assert!((psin(x) - x.sin()).abs() < 2e-7, "{x}");
        }
    }

    /// Disengaged, the input comes out three samples late.
    #[test]
    fn disengaged_passes_three_samples_late() {
        let input = noise(2000);
        let out = run(SR, &params(&[("touchstrip_on_off", 0.0)]), &input);
        for i in 3..2000 {
            assert_eq!(out[i], input[i - 3], "{i}");
        }
    }

    /// Touched on a tone with the swirl off: the record plays on, slows and stops.
    #[test]
    fn the_record_brakes_to_a_stop() {
        let input: Vec<[f32; 2]> = (0..(SR as usize * 3))
            .map(|i| {
                let v = (std::f32::consts::TAU * 330.0 * i as f32 / SR).sin() * 0.5;
                [v, v]
            })
            .collect();
        let out = run(SR, &params(&[("touchstrip_pos", 0.0), ("scratcher_wet_level", 0.0)]), &input);
        let early = rms(out[2000..6000].iter().map(|x| x[0]));
        let late = rms(out[out.len() - 4000..].iter().map(|x| x[0]));
        assert!(early > 0.1, "plays on at first: {early}");
        assert!(late < 1e-4, "stopped: {late}");
    }

    /// The swirl echoes what the needle plays (it dies away within a fraction of a
    /// second, as Maschine's does).
    #[test]
    fn the_swirl_echoes() {
        let mut input = vec![[0.0f32; 2]; SR as usize / 4];
        input[100] = [0.8, 0.8];
        let level = |wet: f64| {
            let out = run(SR, &params(&[("scratcher_wet_level", wet), ("scratcher_feedback", 0.9)]), &input);
            rms(out[2205..4410].iter().map(|x| x[0]))
        };
        let (dry, wet) = (level(0.0), level(1.0));
        assert!(wet > 10.0 * dry && wet > 1e-6, "{dry} {wet}");
    }

    // Null tests against Maschine 3: renders in out/perform-scratcher/ (the recipe in
    // out/perform-ref-ready.md, `--preroll 0`, 2 s of noise); skipped when absent.

    fn wav(path: &std::path::Path) -> Option<(f32, Vec<[f32; 2]>)> {
        let mut r = hound::WavReader::open(path).ok()?;
        let sr = r.spec().sample_rate as f32;
        let s: Vec<f32> = r.samples::<f32>().map(Result::unwrap).collect();
        Some((sr, s.as_chunks::<2>().0.to_vec()))
    }

    #[test]
    fn nulls_against_maschine() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/perform-scratcher");
        let cases: &[(&str, &[(&str, f64)])] = &[
            ("default", &[]),
            ("off", &[("touchstrip_on_off", 0.0)]),
            ("pos0", &[("touchstrip_pos", 0.0)]),
            ("pos1", &[("touchstrip_pos", 1.0)]),
            ("dry", &[("scratcher_wet_level", 0.0)]),
            (
                "hot",
                &[("scratcher_wet_level", 1.0), ("scratcher_feedback", 1.0), ("scratcher_freq_shift", 0.9), ("scratcher_delay_time", 0.2)],
            ),
            ("low", &[("touchstrip_pos", 0.3), ("scratcher_freq_shift", 0.1), ("scratcher_delay_time", 1.0), ("scratcher_feedback", 0.05)]),
            ("default44100", &[]),
            ("default96000", &[]),
        ];
        let mut report = Vec::new();
        for (name, extra) in cases {
            let (Some((sr, input)), Some((_, want))) = (wav(&dir.join(format!("in-{name}.wav"))), wav(&dir.join(format!("{name}.wav"))))
            else {
                continue;
            };
            let got = run_faded(sr, &params(extra), &input, true);
            let n = got.len().min(want.len());
            let k = skip(sr);
            let mut worst = -240.0f32;
            for ch in 0..2 {
                let diff = rms((k..n).map(|i| got[i][ch] - want[i][ch]));
                if diff > 0.0 {
                    worst = worst.max(20.0 * diff.log10());
                }
            }
            let first = (k..n).find(|&i| got[i] != want[i]);
            report.push((name, worst, first));
        }
        let text: Vec<String> = report.iter().map(|(n, db, f)| format!("{n}: {db:.1} dB, first difference at {f:?}")).collect();
        eprintln!("{}", text.join("\n"));
        assert!(report.iter().all(|(_, db, _)| *db <= -200.0), "{}", text.join("\n"));
    }
}
