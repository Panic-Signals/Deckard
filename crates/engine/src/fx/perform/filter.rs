//! Filter: Maschine's Perform FX filter (docs/fx-perform-filter.md): a nonlinear
//! two-integrator low / band / high pass with resonance feedback, every stage through a
//! soft clip, at twice the sample rate (below 88 kHz). The strip sweeps its cutoff
//! between Min. and Max. Freq in pitch; Saturation drives it, Out Level sets its level.
//! Its glides are linear over 20 ms; engaging fades the wet signal in, and released the
//! signal still runs through the converters (16 samples late).

use super::{Ctx, Mode, Params};

/// `__exp10` from the system math library, which Maschine calls (10^x, f64).
#[cfg(target_os = "macos")]
fn exp10(x: f64) -> f64 {
    unsafe extern "C" {
        fn __exp10(x: f64) -> f64;
    }
    // SAFETY: a pure libm function.
    unsafe { __exp10(x) }
}

#[cfg(not(target_os = "macos"))]
fn exp10(x: f64) -> f64 {
    10f64.powf(x)
}

/// `10^((v·50 + floor)/20)` as the setters work it out (§1).
fn db_gain(v: f32, floor: f64) -> f32 {
    let db = (v.clamp(0.0, 1.0) as f64).mul_add(50.0, floor) as f32;
    exp10(db as f64 * f64::from_bits(0x3fa9_9999_9999_999a)) as f32
}

fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// The converters' 16 taps (docs/fx-saturator.md §5: `sinc(t)·I0(…)/427.564…` at
/// `t = k + ½`, `k = −8..7`).
fn src_coefs() -> [f32; 16] {
    let mut h = [0f32; 16];
    let pi = f64::from_bits(0x4009_21fb_5444_2d11);
    for (k, o) in (-8i32..8).zip(h.iter_mut()) {
        let t = 0.5f32 + k as f32;
        let a = t as f64 * pi;
        let s = a.sin();
        let u = ((t + t) * c(0x3d80_0000)) as f64;
        let b = (-u).mul_add(u, 1.0).sqrt() * 8.0;
        let i0 = if b >= 3.75 {
            let y = 3.75 / b;
            let mut p = y.mul_add(f64::from_bits(0x3f70_125e_fd3b_7f88), f64::from_bits(0xbf90_df2b_ca08_4c55));
            for k in [
                0x3f9a_fce6_f0e5_5ae3u64,
                0xbf95_1227_1efd_c716,
                0x3f82_c3f3_8a50_d154,
                0xbf59_d0c1_4e13_894a,
                0x3f62_7548_2b92_d5db,
                0x3f8b_35a5_ff2d_9d1c,
                0x3fd9_8845_3365_de00,
            ] {
                p = y.mul_add(p, f64::from_bits(k));
            }
            b.exp() / b.sqrt() * p
        } else {
            let y = b / 3.75;
            let y = y * y;
            let mut p = y.mul_add(f64::from_bits(0x3f72_c3d7_5ac5_4874), f64::from_bits(0x3fa2_78a8_4988_e5fd));
            for k in [0x3fd1_05b4_74e7_32aau64, 0x3ff3_4ed8_3fc7_962c, 0x4008_b833_b894_30a4, 0x400c_1ffe_e624_8ca7] {
                p = y.mul_add(p, f64::from_bits(k));
            }
            y.mul_add(p, 1.0)
        };
        *o = ((s / a) * i0 / f64::from_bits(0x407a_b906_bec5_d25f)) as f32;
    }
    h
}

/// One input into the converters' transposed 16-tap FIR (their lane order); the finished
/// output.
fn fir_step(r: &mut [f32; 16], h: &[f32; 16], x: f32) -> f32 {
    let p = [x * h[0], x * h[1], x * h[2], x * h[3]];
    let t1 = [r[0] + p[0], r[1] + p[1], r[2] + p[2], r[3] + p[3]];
    let t2 = [p[0] + r[4], p[1] + r[5], p[2] + r[6], p[3] + r[7]];
    let q = [x * h[4], x * h[5], x * h[6], x * h[7]];
    let t3 = [r[8] + q[0], r[9] + q[1], r[10] + q[2], r[11] + q[3]];
    let t4 = [q[0] + r[12], q[1] + r[13], q[2] + r[14], q[3] + r[15]];
    r[0..4].copy_from_slice(&[t1[1], t1[2], t1[3], t1[0]]);
    r[4..8].copy_from_slice(&[t2[3], t2[0], t2[1], t2[2]]);
    r[8..12].copy_from_slice(&[t3[1], t3[2], t3[3], t3[0]]);
    r[12..16].copy_from_slice(&[t4[3], t4[0], t4[1], t4[2]]);
    let out = t1[0];
    r[3] = r[11];
    r[11] = r[12];
    r[12] = r[4];
    r[4] = 0.0;
    out
}

/// The ×2 up-converter, one channel, a sample at a time: the input 8 samples ago, then
/// the FIR of this one.
#[derive(Clone, Default)]
struct Up {
    delay: [f32; 8],
    at: usize,
    acc: [f32; 16],
}

impl Up {
    fn push(&mut self, h: &[f32; 16], x: f32) -> (f32, f32) {
        let even = self.delay[self.at];
        self.delay[self.at] = x;
        self.at = (self.at + 1) % 8;
        (even, fir_step(&mut self.acc, h, x))
    }
}

/// The ×½ down-converter, one channel: per pair, `½·(the even input 8 pairs ago + the
/// FIR of the odd inputs so far)`.
#[derive(Clone, Default)]
struct Down {
    delay: [f32; 8],
    at: usize,
    acc: [f32; 16],
    sum: f32,
}

impl Down {
    fn push(&mut self, h: &[f32; 16], even: f32, odd: f32) -> f32 {
        let d = self.delay[self.at];
        self.delay[self.at] = even;
        self.at = (self.at + 1) % 8;
        self.sum += d;
        let out = self.sum * 0.5;
        self.sum = fir_step(&mut self.acc, h, odd);
        out
    }
}

/// A linear glide (§3): to its target in `len` steps, then snapping there.
#[derive(Clone, Copy, Default)]
struct Glide {
    value: f32,
    target: f32,
    step: f32,
    len: f32,
    counter: i32,
    reload: i32,
    pending: bool,
}

impl Glide {
    fn set(&mut self, t: f32) {
        self.target = t;
        if t != self.value {
            self.step = (t - self.value) / self.len;
            self.counter = self.reload;
            self.pending = true;
        }
    }

    fn snap(&mut self) {
        if self.pending {
            self.value = self.target;
            self.step = 0.0;
            self.counter = 0;
            self.pending = false;
        }
    }

    fn tick(&mut self) {
        let arrived = (self.target - self.value).abs() <= 0.0 || {
            self.counter -= 1;
            self.counter <= 0
        };
        if arrived {
            self.snap();
        } else {
            self.value += self.step;
        }
    }
}

const CUTOFF: usize = 0;
const RES: usize = 1;
const DRIVE: usize = 2;
const ENGAGE: usize = 3;
const OUT: usize = 4;
const FADE: usize = 5;

/// `L·tanh(v/L)` the core's way (§4.2): `t = s·v`.
fn clip(v: f32, s: f32, l: f32) -> f32 {
    let t = s * v;
    if ((t.abs()) as f64) < 0.001 {
        return t * l;
    }
    if t.abs() >= 10.0 {
        return l.copysign(v);
    }
    let y = t * c(0x4038_aa3b);
    let i = y.round_ties_even();
    let i = if i < 2_147_483_648.0 && i > -2_147_483_648.0 { i as i32 } else { i32::MIN };
    let f = y - i as f32;
    let p = f.mul_add(c(0x3d65_6042), c(0x3e77_ced9));
    let p = p.mul_add(f, c(0x3f31_6873));
    let p = p.mul_add(f, 1.0);
    let e = p * f32::from_bits((0x3f80_0000i32).wrapping_add(i.wrapping_shl(23)) as u32);
    ((e + -1.0) / (e + 1.0)) * l
}

/// The core's arctangent (§4.1).
fn atan_approx(w: f32) -> f32 {
    const K: [u32; 7] = [0xbc84_6dee, 0x3d2f_c1fa, 0xbd9a_316f, 0x3dda_3db4, 0xbe11_7fc7, 0x3e4c_bc06, 0xbeaa_aa5c];
    let x = if w > c(0x3fc1_14e4) { c(0x3fc1_14e4) } else { w };
    let a = |r: f32| {
        let q = r * r;
        let mut s = r * c(0x3b3b_d75c) + c(K[0]);
        for k in &K[1..] {
            s = q * s + c(*k);
        }
        r * (q * s + 1.0)
    };
    if x > 1.0 {
        c(0x3fc9_0ff9) - a(1.0 / x)
    } else if x < -1.0 {
        c(0xbfc9_0ff9) - a(1.0 / x)
    } else {
        a(x)
    }
}

/// `f64(a) + f64(m)·f64(b − a)`, fused in f64, as f32.
fn mix(a: f32, b: f32, m: f32) -> f32 {
    (m as f64).mul_add((b - a) as f64, a as f64) as f32
}

/// One channel's states (§4.3).
#[derive(Clone, Copy, Default)]
struct Chan {
    y1: f32,
    y2: f32,
    y3: f32,
    y4: f32,
    xp: f32,
    /// The high pass's one-pole stage.
    z: f32,
}

const L1: f32 = 4.0;
const S1: f32 = 0.25;
const L2: f32 = 3.4;
const M: f32 = 0.2;

fn s2() -> f32 {
    c(0x3e96_9696)
}

pub struct Filter {
    /// Oversampling factor (2 below 88 kHz).
    os: f32,
    t: f32,
    h: [f32; 16],
    up: [Up; 2],
    down: [Down; 2],
    glides: [Glide; 6],
    /// The strip position and resonance as set (read back by the setters).
    pos: f32,
    res: f32,
    pmin: f32,
    pmax: f32,
    kind: usize,
    next: usize,
    /// The engage glide's last value (0: changes are instant).
    engaged: f32,
    /// The cutoff and resonance the coefficients were made for.
    made_f: f32,
    made_r: f32,
    k: f32,
    g: f32,
    b1: f32,
    b2: f32,
    ig: f32,
    hp_h: f32,
    hp_c: f32,
    ch: [Chan; 2],
    noise: u32,
    /// The parameters as last set (index order, §1); `None` before the first.
    last: Option<[f32; 8]>,
}

impl Filter {
    pub fn new(sr: f32) -> Filter {
        let os = if sr < 88_000.0 { 2.0 } else { 1.0 };
        let len = ((sr * os) * c(0x3a83_126f)) * 20.0;
        let glide = Glide { len, reload: len.round() as i32, ..Glide::default() };
        Filter {
            os,
            t: 1.0 / (os * sr),
            h: src_coefs(),
            up: Default::default(),
            down: Default::default(),
            glides: [glide; 6],
            pos: 0.0,
            res: 0.0,
            pmin: 20.0,
            pmax: 100.0,
            kind: 0,
            next: 0,
            engaged: 0.0,
            made_f: -1.0,
            made_r: -1.0,
            k: 0.0,
            g: -1.0,
            b1: 0.0,
            b2: 0.0,
            ig: 0.0,
            hp_h: 0.0,
            hp_c: 0.0,
            ch: [Chan::default(); 2],
            noise: 0,
            last: None,
        }
    }

    fn snap_all(&mut self) {
        for g in &mut self.glides {
            g.snap();
        }
    }

    fn clear(&mut self) {
        self.ch = [Chan::default(); 2];
    }

    /// The cutoff (Hz) for the strip position (§1.1).
    fn cutoff(&self) -> f32 {
        let p = self.pos.clamp(0.0, 1.0) as f64;
        let pitch = p.mul_add((self.pmax - self.pmin) as f64, self.pmin as f64) as f32;
        (pitch as f64).mul_add(f64::from_bits(0x3fb5_5555_5555_5555), f64::from_bits(0x4008_4039_7fa8_81bf)).exp2() as f32
    }

    /// The resonance glide's target (§1.2), from the cutoff's.
    fn set_res(&mut self) {
        let mut r = self.res * c(0x3f99_999a);
        r = r * if r < 1.0 { r.abs().mul_add(c(0xbf66_6666), c(0x3ff3_3333)) } else { 1.0 };
        let f = self.glides[CUTOFF].target;
        if r > c(0x3f19_999a) && f > 16_000.0 {
            let x = ((f + -16_000.0) / 8000.0).clamp(0.0, 1.0) as f64;
            r = x.mul_add((c(0x3f19_999a) - r) as f64, r as f64) as f32;
        }
        self.glides[RES].set(r);
        if self.engaged == 0.0 {
            self.snap_all();
        }
    }

    /// The cutoff glide's target again, snapped while released.
    fn set_cutoff(&mut self) {
        let f = self.cutoff();
        self.glides[CUTOFF].set(f);
        if self.engaged == 0.0 {
            self.glides[CUTOFF].snap();
        }
    }

    /// `setParameter(i, v)` (§1, §3.2).
    fn set_param(&mut self, i: usize, v: f32) {
        match i {
            0 => {
                self.pos = v;
                self.set_cutoff();
                self.set_res();
            }
            1 => {
                let on = v > 0.5;
                self.glides[ENGAGE].set(if on { 1.0 } else { 0.0 });
                if on && self.engaged == 0.0 {
                    self.clear();
                }
            }
            2 | 3 => {
                let v = v.clamp(0.0, 1.0) as f64;
                if i == 2 {
                    self.pmin = v.mul_add(50.0, 10.0) as f32;
                } else {
                    self.pmax = v.mul_add(30.0, 110.0) as f32;
                }
                self.set_cutoff();
                self.set_res();
            }
            4 => {
                self.res = v;
                self.set_res();
            }
            5 => {
                let t = if v < 0.5 {
                    0
                } else if v < 1.5 {
                    1
                } else {
                    2
                };
                if self.kind != t {
                    self.next = t;
                    self.glides[FADE].set(0.0);
                }
            }
            6 => {
                self.glides[DRIVE].set(db_gain(v, 0.0));
                if self.engaged == 0.0 {
                    self.snap_all();
                }
            }
            _ => {
                self.glides[OUT].set(db_gain(v, -50.0));
                if self.engaged == 0.0 {
                    self.snap_all();
                }
            }
        }
    }

    /// The coefficients for the glided cutoff and resonance, where they moved (§4.1).
    fn coefficients(&mut self) {
        let f = self.glides[CUTOFF].value;
        if f != self.made_f {
            self.made_f = f;
            self.tsk(f);
            let w = ((f * 0.5) * c(0x4049_0fdb)) * self.t;
            let w2 = w * w;
            let w3 = w * w2;
            let w5 = w2 * w3;
            let h = w5.mul_add(c(0x3e08_8889), w3.mul_add(c(0x3eaa_aaab), w));
            self.hp_h = h;
            self.hp_c = 1.0 / (h + 1.0);
        }
        let r = self.glides[RES].value;
        if r != self.made_r {
            self.made_r = r;
            self.k = if r < 2.0 { r + r } else { 4.0 };
            self.tsk(self.made_f);
        }
    }

    fn tsk(&mut self, f: f32) {
        let g = atan_approx((f * c(0x4049_0fdb)) * self.t);
        self.g = g;
        let g1 = g + 1.0;
        self.b1 = (g * g1) / g.mul_add((2.0 - self.k) + g, 1.0);
        self.b2 = g / g1;
        self.ig = 1.0 / g;
    }

    /// One oversampled sample through the core (§4.3).
    fn core(&mut self, ch: usize, x: f32) {
        let (k, ig, b1, b2, g) = (self.k, self.ig, self.b1, self.b2, self.g);
        let s = &mut self.ch[ch];
        let (y1, y2, y3, y4) = (s.y1, s.y2, s.y3, s.y4);
        let u = ig.mul_add(y2, y3 - y4);
        let v1 = b2 * u;
        let a1 = clip(v1, S1, L1);
        let bb1 = clip((-k).mul_add(a1, x), s2(), L2);
        let bb2 = clip(k.mul_add(mix(y1 - y2, y3 - y4, M), s.xp), s2(), L2);
        let v2 = b1 * ig.mul_add(y1, (bb1 + bb2) - y3);
        let a2 = clip(v2, S1, L1);
        let v3 = b2 * ig.mul_add(y2, (y3 + a2) - y4);
        let a3 = clip(v3, S1, L1);
        let bb3 = clip(k.mul_add(mix(v2 - v3, a2 - a3, M), x), s2(), L2);
        let n1 = g.mul_add(((bb3 + bb2) - y3) - a2, y1);
        let n3 = clip(n1, S1, L1);
        let n2 = g.mul_add(((y3 + n3) - y4) - a3, y2);
        let n4 = clip(n2, S1, L1);
        *s = Chan { y1: n1, y2: n2, y3: n3, y4: n4, xp: x, z: s.z };
    }

    /// The mode's output for channel `ch` (§4.3).
    fn output(&mut self, ch: usize) -> f32 {
        let (k, h, cc) = (self.k, self.hp_h, self.hp_c);
        let s = &mut self.ch[ch];
        match self.kind {
            0 => s.y2,
            1 => (s.y3 - s.y4) * 1.5,
            _ => {
                let b = clip(k.mul_add(mix(s.y1 - s.y2, s.y3 - s.y4, M), s.xp), s2(), L2);
                let u = b - s.y3;
                let o = u + cc * (-u).mul_add(h, s.z * -2.0);
                s.z = o.mul_add(h, s.z);
                o
            }
        }
    }

    /// One oversampled sample, both channels (§2 step 2).
    fn step(&mut self, x: [f32; 2]) -> [f32; 2] {
        for g in &mut self.glides {
            g.tick();
        }
        let fade = self.glides[FADE].value;
        if fade == 0.0 {
            self.kind = self.next;
            self.glides[FADE].set(1.0);
            self.clear();
        }
        let n1 = self.noise.wrapping_mul(0x19fb_4787).wrapping_add(0x16d1_b86b);
        let n2 = n1.wrapping_mul(0x19fb_4787).wrapping_add(0x16d1_b86b);
        self.noise = n2;
        let n = |s: u32| f32::from_bits(0x4000_0000 | s >> 9) + -3.0;
        self.coefficients();
        let xs = [n(n1).mul_add(c(0x0da2_4260), x[0]), n(n2).mul_add(c(0x0da2_4260), x[1])];
        let gain = self.glides[DRIVE].value * 0.5;
        self.core(0, xs[0] * gain);
        self.core(1, gain * xs[1]);
        let out = self.glides[OUT].value;
        let on = self.glides[ENGAGE].value;
        let mut o = [0.0; 2];
        for ch in 0..2 {
            let y = (self.output(ch) * fade) * (out + out);
            o[ch] = on.mul_add(y - xs[ch], xs[ch]);
        }
        self.engaged = on;
        o
    }
}

impl Mode for Filter {
    fn set(&mut self, p: &Params) {
        let now = [
            p.f("touchstrip_pos"),
            if p.f("touchstrip_on_off") >= 0.5 { 1.0 } else { 0.0 },
            p.f("filter_min_freq"),
            p.f("filter_max_freq"),
            p.f("filter_resonance"),
            p.f("filter_type"),
            p.f("filter_saturation"),
            p.f("filter_out_level"),
        ];
        match self.last {
            // A new effect: every parameter set while suspended (all snap), then resumed
            // (the wet signal fades in) (§1.3, §3.3).
            None => {
                for (i, v) in now.into_iter().enumerate() {
                    self.set_param(i, v);
                    self.snap_all();
                }
                self.kind = self.next;
                let fade = &mut self.glides[FADE];
                (fade.value, fade.target, fade.step, fade.counter) = (0.0, 0.0, 0.0, 0);
                self.made_f = -1.0;
                self.made_r = -1.0;
            }
            Some(was) => {
                for (i, v) in now.into_iter().enumerate() {
                    if v != was[i] {
                        self.set_param(i, v);
                    }
                }
            }
        }
        self.last = Some(now);
    }

    fn frame(&mut self, l: f32, r: f32, _c: &Ctx) -> (f32, f32) {
        if self.os == 1.0 {
            let o = self.step([l, r]);
            return (o[0], o[1]);
        }
        let h = self.h;
        let (el, ol) = self.up[0].push(&h, l);
        let (er, or) = self.up[1].push(&h, r);
        let a = self.step([el, er]);
        let b = self.step([ol, or]);
        (self.down[0].push(&h, a[0], b[0]), self.down[1].push(&h, a[1], b[1]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44_100.0;

    fn params(kv: &[(&str, f64)]) -> Vec<(String, f64)> {
        let mut p: Vec<(String, f64)> = deckard_model::PERFORM_FX.1.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        for (k, v) in kv {
            match p.iter_mut().find(|(n, _)| n == k) {
                Some(x) => x.1 = *v,
                None => p.push((k.to_string(), *v)),
            }
        }
        p
    }

    fn ctx(p: &[(String, f64)]) -> Ctx {
        let v = |k: &str| p.iter().find(|(n, _)| n == k).map_or(0.0, |x| x.1) as f32;
        Ctx { sr: SR, pos: v("touchstrip_pos"), engaged: v("touchstrip_on_off") >= 0.5, quarter: SR / 2.0, song: None }
    }

    fn run(f: &mut Filter, p: &[(String, f64)], l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let c = ctx(p);
        l.iter().zip(r).map(|(&a, &b)| f.frame(a, b, &c)).unzip()
    }

    fn sine(hz: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (std::f32::consts::TAU * hz * i as f32 / SR).sin() * 0.5).collect()
    }

    /// Released, Maschine's filter still runs the signal through its 2× converters: it comes
    /// out 16 samples late, otherwise (well below Nyquist) the same.
    #[test]
    fn released_it_passes_through_the_converters() {
        let p = params(&[("touchstrip_on_off", 0.0)]);
        let mut f = Filter::new(SR);
        f.set(&Params(&p));
        let x = sine(200.0, 4_000);
        let (l, _) = run(&mut f, &p, &x, &x);
        let err = (1_000..4_000).map(|i| (l[i] - x[i - 16]).abs()).fold(0.0, f32::max);
        assert!(err < 1e-3, "{err}");
    }

    /// A float WAV's frames (as maschine-ref writes them), or None if it isn't there.
    fn wav(path: &str) -> Option<(Vec<[f32; 2]>, f32)> {
        let b = std::fs::read(path).ok()?;
        let (mut i, mut sr, mut ch) = (12, 0u32, 2usize);
        while i + 8 <= b.len() {
            let id = &b[i..i + 4];
            let n = u32::from_le_bytes(b[i + 4..i + 8].try_into().ok()?) as usize;
            if id == b"fmt " {
                ch = u16::from_le_bytes(b[i + 10..i + 12].try_into().ok()?) as usize;
                sr = u32::from_le_bytes(b[i + 12..i + 16].try_into().ok()?);
            }
            if id == b"data" {
                let v: Vec<f32> = b[i + 8..i + 8 + n].as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
                return Some((v.chunks_exact(ch).map(|f| [f[0], f[ch - 1]]).collect(), sr as f32));
            }
            i += 8 + n + (n & 1);
        }
        None
    }

    /// Maschine's renders (out/perform-filter/, docs/fx-perform-filter.md §5): DECKARD's
    /// filter, after the same second of silence, gives the same samples (but the noise,
    /// about 600 dB down). Skipped where the renders aren't there.
    #[test]
    fn nulls_against_maschine() {
        let cases: &[(&str, &[(&str, f64)])] = &[
            ("lp", &[]),
            ("lp44", &[]),
            ("bp", &[("filter_type", 1.0)]),
            ("hp", &[("filter_type", 2.0)]),
            ("off", &[("touchstrip_on_off", 0.0), ("filter_resonance", 1.0)]),
            ("pos0", &[("touchstrip_pos", 0.0)]),
            ("sat1", &[("filter_saturation", 1.0), ("filter_out_level", 0.2)]),
            ("minmax", &[("filter_min_freq", 1.0), ("filter_max_freq", 0.0), ("touchstrip_pos", 0.7), ("filter_type", 1.0)]),
            ("out1", &[("filter_out_level", 1.0), ("filter_saturation", 0.0)]),
            ("r88", &[]),
            ("r96lp", &[("touchstrip_pos", 0.3), ("filter_resonance", 0.5)]),
            ("hfres", &[("touchstrip_pos", 0.95), ("filter_resonance", 0.8)]),
            ("hfres2", &[("touchstrip_pos", 1.0), ("filter_max_freq", 1.0), ("filter_resonance", 0.75), ("filter_type", 1.0)]),
            ("res7", &[("filter_resonance", 0.7), ("touchstrip_pos", 0.4)]),
            ("res7hp", &[("filter_resonance", 0.7), ("filter_type", 2.0), ("touchstrip_pos", 0.2)]),
            ("sat9", &[("filter_saturation", 0.9), ("filter_resonance", 0.6)]),
            ("loudhp2", &[("filter_type", 2.0), ("filter_saturation", 0.8), ("filter_resonance", 0.5)]),
        ];
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../out/perform-filter");
        for (name, kv) in cases {
            let (Some((input, sr)), Some((want, _))) = (wav(&format!("{dir}/{name}-in.wav")), wav(&format!("{dir}/{name}.wav"))) else {
                continue;
            };
            let p = params(&[&[("touchstrip_on_off", 1.0)], *kv].concat());
            let mut f = Filter::new(sr);
            f.set(&Params(&p));
            let c = ctx(&p);
            for _ in 0..sr as usize {
                f.frame(0.0, 0.0, &c);
            }
            let worst = input.iter().zip(&want).map(|(x, w)| {
                let (l, r) = f.frame(x[0], x[1], &c);
                (l - w[0]).abs().max((r - w[1]).abs())
            });
            let worst = worst.fold(0.0f32, f32::max);
            assert!(worst < 1e-20, "{name}: {worst:e}");
        }
    }

    /// Engaging glides the effect in over 20 ms of oversampled samples (1764 at 44.1 kHz),
    /// then lands exactly; releasing glides it out the same way (§3).
    #[test]
    fn engaging_glides_over_20_ms() {
        let off = params(&[("touchstrip_on_off", 0.0)]);
        let on = params(&[("touchstrip_on_off", 1.0)]);
        let mut f = Filter::new(SR);
        f.set(&Params(&off));
        let c = ctx(&on);
        for _ in 0..100 {
            f.frame(0.1, 0.1, &c);
        }
        f.set(&Params(&on));
        let mut seen = Vec::new();
        for _ in 0..1764 / 2 + 1 {
            f.frame(0.1, 0.1, &c);
            seen.push(f.glides[ENGAGE].value);
        }
        assert!(seen.windows(2).all(|w| w[1] >= w[0]), "rising");
        assert!(seen[seen.len() - 3] < 1.0, "still gliding just before 20 ms");
        assert_eq!(*seen.last().unwrap(), 1.0, "landed at 20 ms");
        f.set(&Params(&off));
        for _ in 0..1764 / 2 + 1 {
            f.frame(0.1, 0.1, &c);
        }
        assert_eq!(f.glides[ENGAGE].value, 0.0);
    }

    /// Released, a parameter change takes effect at once (nothing to hear glide); engaged,
    /// it glides.
    #[test]
    fn released_changes_are_instant() {
        let mut f = Filter::new(SR);
        f.set(&Params(&params(&[("touchstrip_on_off", 0.0), ("filter_saturation", 0.0)])));
        let c = ctx(&params(&[]));
        f.frame(0.0, 0.0, &c);
        f.set(&Params(&params(&[("touchstrip_on_off", 0.0), ("filter_saturation", 1.0)])));
        assert_eq!(f.glides[DRIVE].value, f.glides[DRIVE].target, "snapped");
        let mut g = Filter::new(SR);
        g.set(&Params(&params(&[("touchstrip_on_off", 1.0), ("filter_saturation", 0.0)])));
        for _ in 0..2000 {
            g.frame(0.0, 0.0, &c);
        }
        g.set(&Params(&params(&[("touchstrip_on_off", 1.0), ("filter_saturation", 1.0)])));
        assert!(g.glides[DRIVE].value < g.glides[DRIVE].target, "glides while engaged");
    }
}
