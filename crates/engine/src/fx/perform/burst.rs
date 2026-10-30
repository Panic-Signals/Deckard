//! Burst Echo: Maschine's `NI::DSP::JamBurstEcho`, written from its behaviour as
//! docs/fx-perform-burst-echo.md describes it. Engaging opens the input to a three-unit
//! "tape" echo for a burst (Feed long, or all the while engaged at the top of Feed); the
//! repeats run through a −10 dB high shelf and a high pass (Filter), a soft clip in each
//! tape write, an output low pass and a gentle limiter. The strip sets the feedback or
//! the time (TS Assign); released, the feedback falls to at most 0.72 and the echoes
//! fade. Mix keeps the dry signal, Replace mutes it after the first repeat, Wet only
//! drops it.

use super::super::exp_e;
use super::{Ctx, Mode, Params};

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

/// `10^x` in double (Apple's `exp10`).
fn exp10(x: f64) -> f64 {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn __exp10(x: f64) -> f64;
        }
        unsafe { __exp10(x) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        10f64.powf(x)
    }
}

const PI32: f32 = std::f32::consts::PI;
const PI64: f64 = std::f64::consts::PI;
/// The control-rate block: the Perform FX processor ticks its effect every 128 samples.
const BLOCK: usize = 128;
/// 0.99 and 0.72 as Maschine's floats.
const P99: f32 = f32::from_bits(0x3f7d_70a4);
const FB_OFF: f32 = f32::from_bits(0x3f38_51ec);

/// tan(x) by its series to x⁵ (§2.3).
fn tan3(x: f32) -> f32 {
    let x2 = x * x;
    let x3 = x * x2;
    let x5 = x2 * x3;
    let g = x3.mul_add(f32::from_bits(0x3eaa_aaab), x);
    x5.mul_add(f32::from_bits(0x3e08_8889), g)
}

/// A trapezoidal state-variable filter (§2.3).
#[derive(Default, Clone, Copy)]
struct Svf {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl Svf {
    /// (band, low).
    fn tick(&mut self, x: f32) -> (f32, f32) {
        let v3 = x - self.ic2;
        let v1 = self.a1.mul_add(self.ic1, v3 * self.a2);
        let v2 = self.ic2 + self.a2.mul_add(self.ic1, v3 * self.a3);
        self.ic1 = v1 * 2.0 - self.ic1;
        self.ic2 = v2 * 2.0 - self.ic2;
        (v1, v2)
    }

    fn set(&mut self, g: f32, k: f32) {
        self.a1 = 1.0 / g.mul_add(k + g, 1.0);
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    fn clear(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }
}

/// Eigen's vectorised sine (`sine`) or cosine, one lane (§2.4).
fn sincos(x: f32, sine: bool) -> f32 {
    let xa = x.abs();
    let y = xa * f32::from_bits(0x3f22_f983) + 12_582_912.0;
    let yi = y.to_bits();
    let yr = y + -12_582_912.0;
    let mut r = yr.mul_add(f32::from_bits(0xbfc9_0fd8), xa);
    r = yr.mul_add(f32::from_bits(0xb4a8_885a), r);
    r = yr.mul_add(f32::from_bits(0xa7c2_34c4), r);
    let x2 = r * r;
    let mut c = x2.mul_add(f32::from_bits(0x37cc_730b), f32::from_bits(0xbab6_036e));
    c = x2.mul_add(c, f32::from_bits(0x3d2a_aa9e));
    c = x2.mul_add(c, -0.5);
    c = x2.mul_add(c, 1.0);
    let mut s = x2.mul_add(f32::from_bits(0xb94d_70ca), f32::from_bits(0x3c08_85d3));
    s = x2.mul_add(s, f32::from_bits(0xbe2a_aaa8));
    let s = r.mul_add(x2 * s, r);
    let even = yi & 1 == 0;
    let (v, sign) = if sine {
        (if even { s } else { c }, ((yi << 30) ^ x.to_bits()) & 0x8000_0000)
    } else {
        (if even { c } else { s }, (yi << 30).wrapping_add(0x4000_0000) & 0x8000_0000)
    };
    f32::from_bits(v.to_bits() ^ sign)
}

/// Σ k[i]·trig(w[i]·x), summed as (0+2)+(1+3).
fn trig_sum(k: &[f32; 4], w: &[f32; 4], x: f32, sine: bool) -> f32 {
    let v: [f32; 4] = std::array::from_fn(|i| sincos(w[i] * x, sine) * k[i]);
    (v[0] + v[2]) + (v[1] + v[3])
}

/// The soft clip in each tape write: x + Σ aₖ sin(kπx) up to ±1, flat beyond, with
/// first-order antiderivative anti-aliasing (§3.3).
#[derive(Clone, Copy)]
struct Clip {
    a: [f32; 4],
    b: [f32; 4],
    w: [f32; 4],
    gain: f32,
    c1: f32,
    c2: f32,
    px: f32,
    pf: f32,
}

impl Clip {
    fn new() -> Clip {
        let a = [0x3f02_6136, 0x3e02_6136, 0x3cc6_ac83, 0x3b15_0162].map(f32::from_bits);
        let b = [0x3e26_012f, 0x3ca6_012f, 0x3b28_a3be, 0x393d_b835].map(f32::from_bits);
        let w = [0x4049_0fdb, 0x40c9_0fdb, 0x4116_cbe4, 0x4149_0fdb].map(f32::from_bits);
        let c2 = 0.0 - trig_sum(&b, &w, 0.0, false);
        let c1 = ((0.5 - trig_sum(&b, &w, 1.0, false)) - c2) + -1.0;
        let aw: [f32; 4] = std::array::from_fn(|i| a[i] * w[i]);
        let gain = 1.0 / (((aw[0] + aw[2]) + (aw[1] + aw[3])) + 1.0);
        Clip { a, b, w, gain, c1, c2, px: 0.0, pf: 0.0 }
    }

    fn tick(&mut self, input: f32) -> f32 {
        let x = self.gain * input;
        let f = if x > 1.0 {
            x + self.c1
        } else if x < -1.0 {
            self.c1 - x
        } else {
            ((x * x) * 0.5 - trig_sum(&self.b, &self.w, x, false)) - self.c2
        };
        let d = x - self.px;
        let y = if d.abs() < f32::from_bits(0x38d1_b717) {
            let xc = if x < -1.0 { -1.0 } else { x };
            let xc = if xc > 1.0 { 1.0 } else { xc };
            xc + trig_sum(&self.a, &self.w, xc, true)
        } else {
            (f - self.pf) / d
        };
        self.pf = f;
        self.px = x;
        y
    }
}

/// One channel of `NI::DSP::DelayLineVariableSpeed`: a ring the write head moves along at
/// a variable speed (slots per sample), read a fixed distance behind it (§3.4).
struct Line {
    buf: Vec<f32>,
    mask: i32,
    pos: f64,
    idx: i32,
    frac: f64,
    count: i32,
    rpos: f64,
    ridx: i32,
    roff: f64,
    speed: f64,
    /// Speeds of the last three steps, newest first.
    hist: [f64; 3],
    /// The last written values, newest first.
    h: [f32; 2],
    /// The read side's anti-alias low pass, run over the slots the reader skips.
    svf: Svf,
}

impl Line {
    fn new(sr: f32) -> Line {
        let size = ((sr as i32).max(1) as u32).next_power_of_two() as usize;
        Line {
            buf: vec![0.0; size],
            mask: size as i32 - 1,
            pos: 0.0,
            idx: 0,
            frac: 0.0,
            count: 0,
            rpos: 0.0,
            ridx: 0,
            roff: 0.0,
            speed: 0.0,
            hist: [1.0; 3],
            h: [0.0; 2],
            svf: Svf::default(),
        }
    }

    fn reset(&mut self) {
        let buf = std::mem::take(&mut self.buf);
        *self = Line { buf, ..Line::new(1.0) };
        self.mask = self.buf.len() as i32 - 1;
        self.buf.iter_mut().for_each(|v| *v = 0.0);
    }

    fn at(&self, i: i32) -> usize {
        (i & self.mask) as usize
    }

    /// `readLinInterp(offset)`.
    fn read(&mut self, offset: f64) -> f32 {
        let mut d1 = self.pos - offset;
        if d1 < 0.0 {
            d1 += (self.mask + 1) as f64;
        }
        let d2 = self.speed + (self.roff - offset);
        let r = 1.0 / d2.abs();
        let r = if r < 0.2 { 0.2 } else { r };
        let r = if r > 1.0 { 1.0 } else { r } as f32;
        self.svf.set(tan3(0.5 * (r * PI32)), 1.4);
        let i = d1 as i32;
        let n = (self.rpos + d2) as i32 - self.ridx;
        if n > 0 {
            let n = n.min(15);
            for j in (i - n + 3..).take((n + (3 - n).max(2) - 2) as usize) {
                let at = self.at(j);
                self.buf[at] = self.svf.tick(self.buf[at]).1;
            }
        }
        let fr = (d1 - i as f64) as f32;
        self.roff = offset;
        self.ridx = i & self.mask;
        self.rpos = fr as f64 + self.ridx as f64;
        let a = self.buf[self.ridx as usize];
        let b = self.buf[self.at(self.ridx + 1)];
        fr.mul_add(b - a, a)
    }

    /// The write head's step at `s` slots per sample.
    fn advance(&mut self, s: f64) {
        self.speed = s;
        let p = s + self.pos;
        let i = p as i32;
        self.count = i - self.idx;
        self.frac = p - i as f64;
        self.idx = self.mask & i;
        self.pos = self.frac + self.idx as f64;
        self.hist = [s, self.hist[0], self.hist[1]];
    }

    /// Fill the slots the head passed with a parabola through the last three values.
    fn write(&mut self, h0: f32) {
        let [x1, x2] = self.h;
        self.h = [h0, x1];
        let n = self.count;
        if n < 1 {
            return;
        }
        let s0 = self.hist[0] as f32;
        let s4 = -s0;
        let s3 = (s4 as f64 - self.hist[1]) as f32;
        let s5 = h0 / -(s0 * s3);
        let s4c = x1 / -(s0 * (s4 - s3));
        let s6 = x2 / ((s0 + s3) * s3);
        let t = (-s5).mul_add(s3 - s0, -(s4c * s3));
        let b = s6.mul_add(s0, t);
        let a = s6 + (s5 + s4c);
        for k in 0..n {
            let tk = (self.frac + k as f64) as f32;
            let at = self.at(self.idx - k);
            self.buf[at] = (-tk).mul_add((-tk).mul_add(a, b), h0);
        }
    }
}

/// One of the core's three tape units: two lines (L, R), each written through the soft
/// clip, a 10 kHz one-pole and a low pass that follows the tape speed (§3.2).
struct Unit {
    sr: f64,
    len: f64,
    delay: [f64; 2],
    speed: [f64; 2],
    lines: [Line; 2],
    clip: [Clip; 2],
    smooth: [f32; 2],
    smooth_c: f32,
    wsvf: [Svf; 2],
}

impl Unit {
    fn new(sr: f32) -> Unit {
        let sr64 = sr as f64;
        let mut wsvf = [Svf::default(); 2];
        wsvf[0].set(tan3((62_831.853_071_795_864 / sr64) as f32), 1.4);
        wsvf[1].set(tan3(62_831.85 / sr), 1.4);
        Unit {
            sr: sr64,
            len: sr64.max(1.0),
            delay: [0.0; 2],
            speed: [0.0; 2],
            lines: [Line::new(sr), Line::new(sr)],
            clip: [Clip::new(); 2],
            smooth: [0.0; 2],
            smooth_c: (-62_831.853_071_795_864 / sr64).exp() as f32,
            wsvf,
        }
    }

    fn reset(&mut self) {
        self.lines.iter_mut().for_each(Line::reset);
        self.wsvf.iter_mut().for_each(Svf::clear);
        self.smooth = [0.0; 2];
    }

    /// The delay `ms` spread by `sp`: L × (1 + sp/2), R × (1 − sp/2).
    fn set_delay(&mut self, ms: f32, sp: f64) {
        let base = self.sr * ms as f64;
        self.delay = [base * sp.mul_add(0.5, 1.0), base * (-sp).mul_add(0.5, 1.0)];
        self.len = (self.sr * 1.0).max(1.0);
        self.speed = [self.len / self.delay[0], self.len / self.delay[1]];
    }

    fn write(&mut self, l: f32, r: f32) {
        for (ch, x) in [l, r].into_iter().enumerate() {
            let y = self.clip[ch].tick(x);
            let y = self.smooth_c.mul_add(self.smooth[ch] - y, y);
            self.smooth[ch] = y;
            let v2 = self.wsvf[ch].tick(y).1;
            self.lines[ch].write(v2);
        }
    }

    fn read(&mut self) -> (f32, f32) {
        let l = self.lines[0].read(self.len);
        let r = self.lines[1].read(self.len);
        for ch in 0..2 {
            let s = if self.speed[ch] > 50.5 { 50.5 } else { self.speed[ch] };
            self.lines[ch].advance(s);
            let lim = f64::from_bits(0x3fef_ffff_ca50_1acb);
            let f = s * 0.8;
            let f = if f > lim { lim } else { f } as f32;
            self.wsvf[ch].set(tan3((f * PI32) * 0.5), 1.4);
        }
        (l, r)
    }
}

/// The repeats' filters: a −10 dB high shelf, then a 12 dB/oct high pass (§3.5).
struct Filters {
    sr: f32,
    shelf: [Svf; 2],
    hp: [Svf; 2],
    m: [f32; 3],
}

impl Filters {
    fn new(sr: f32) -> Filters {
        Filters { sr, shelf: [Svf::default(); 2], hp: [Svf::default(); 2], m: [0.0; 3] }
    }

    fn set_hp(&mut self, f: f32) {
        let g = tan3(((f as f64 * PI64) / self.sr as f64) as f32);
        self.hp.iter_mut().for_each(|s| s.set(g, 2.0));
    }

    fn set_shelf(&mut self, f: f32) {
        let a = exp10f(-0.25);
        let g = a.sqrt() * tan3(((f as f64 * PI64) / self.sr as f64) as f32);
        self.shelf.iter_mut().for_each(|s| s.set(g, f32::from_bits(0x3fd5_5555)));
        self.m = [a * a, (a * (1.0 - a)) / f32::from_bits(0x3f19_999a), 1.0 - a * a];
    }

    fn reset(&mut self) {
        self.shelf.iter_mut().chain(self.hp.iter_mut()).for_each(Svf::clear);
    }

    fn process(&mut self, l: &mut f32, r: &mut f32) {
        let [m0, m1, m2] = self.m;
        for (ch, x) in [l, r].into_iter().enumerate() {
            let (v1, v2) = self.shelf[ch].tick(*x);
            let y = m2.mul_add(v2, m0.mul_add(*x, v1 * m1));
            let (h1, h2) = self.hp[ch].tick(y);
            *x = (-2.0f32).mul_add(h1, y) - h2;
        }
    }
}

/// `NI::DSP::Limiter` as the core sets it: 0 dB in and out, no side chain, 3 s release
/// (§3.7).
struct Limiter {
    att: f32,
    rel: f32,
    att2: f32,
    rel2: f32,
    env1: f32,
    env2: f32,
}

impl Limiter {
    fn new(sr: f32) -> Limiter {
        Limiter {
            att: 1.0 - 1.0 / (0.005 * sr),
            rel: 1.0 - 1.0 / (3.0 * sr),
            att2: 1.0 / (-15.0 * sr) + 1.0,
            rel2: 1.0 / (-3.0 * sr) + 1.0,
            env1: 0.0,
            env2: 0.0,
        }
    }

    fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let tiny = f32::from_bits(0x2b8c_bccc);
        let dl = l.mul_add(1.0, tiny).abs();
        let dr = r.mul_add(1.0, tiny).abs();
        let peak = if dl < dr { dr } else { dl };
        let db = peak.ln() * f32::from_bits(0x410a_f968);
        let c = if self.env1 < db { self.att } else { self.rel };
        self.env1 = c.mul_add(self.env1 - db, db);
        let c = if self.env2 < db { self.att2 } else { self.rel2 };
        self.env2 = c.mul_add(self.env2 - db, db);
        let lvl = if self.env1 < self.env2 { self.env2 } else { self.env1 };
        let gr = (0.0 - lvl).min(0.0);
        let g = exp_e(gr.mul_add(f32::from_bits(0x411f_73da), 1200.0));
        let clamp = |x: f32| {
            let x = if x < -2.5 { -2.5 } else { x };
            if x > 2.5 { 2.5 } else { x }
        };
        (clamp(g * l), clamp(g * r))
    }
}

/// `NI::DSP::SmoothingToolbox<9>`: the core's parameters, glided linearly over about
/// 20 ms of control blocks (§3.1). Lanes: 4 high pass, 5 shelf, 6 feedback, 7 time (ms).
struct Lanes {
    cur: [f32; 12],
    delta: [f32; 12],
    target: [f32; 12],
    pending: [f32; 12],
    start: [f32; 12],
    t: [f32; 12],
    rate: f32,
    steps: i32,
    counter: i32,
    state: i32,
    changed: bool,
}

impl Lanes {
    fn new() -> Lanes {
        Lanes {
            cur: [0.0; 12],
            delta: [0.0; 12],
            target: [0.0; 12],
            pending: [0.0; 12],
            start: [0.0; 12],
            t: [0.0; 12],
            rate: 0.0,
            steps: 1,
            counter: 0,
            state: 0,
            changed: false,
        }
    }

    fn start_smooth(&mut self) {
        let mut any = 0.0f32;
        for i in 0..12 {
            self.target[i] = self.pending[i];
            self.start[i] = self.cur[i];
            self.t[i] = 0.0;
            self.delta[i] = self.target[i] - self.cur[i];
            any += self.delta[i].abs();
            let a = self.cur[i].abs();
            let ulp = if a == 0.0 || a.is_infinite() { 0.0 } else { f32::from_bits(a.to_bits() + 1) - a };
            if self.delta[i].abs() < ulp {
                self.delta[i] = 0.0;
                self.cur[i] = self.target[i];
                self.start[i] = self.target[i];
            }
        }
        if any > 0.0 {
            self.counter = self.steps;
            self.state = 2;
        } else {
            self.state = 4;
        }
    }

    /// Everything to the pending values at once.
    fn jump(&mut self) {
        self.target = self.pending;
        self.cur = self.target;
        self.start = self.target;
        self.delta = [0.0; 12];
        self.t = [0.0; 12];
        self.start_smooth();
    }

    fn control(&mut self) {
        if self.changed {
            self.changed = false;
            self.start_smooth();
            return;
        }
        if self.state == 0 {
            return;
        }
        self.counter -= 1;
        if self.counter != 0 {
            if self.state == 2 {
                self.state = 3;
            } else if self.state == 4 {
                self.state = 0;
            }
            return;
        }
        self.cur = self.target;
        self.start = self.target;
        self.delta = [0.0; 12];
        self.t = [0.0; 12];
        self.start_smooth();
    }

    fn tick(&mut self) {
        for i in 0..12 {
            self.t[i] += 1.0;
            self.cur[i] = self.start[i] + self.delta[i] * (self.t[i] * self.rate);
        }
    }

    /// Lane `i` straight to `v` (target, pending, current, start).
    fn put(&mut self, i: usize, v: f32) {
        self.delta[i] = 0.0;
        self.target[i] = v;
        self.pending[i] = v;
        self.cur[i] = v;
        self.start[i] = v;
    }

    /// Lane `i` to glide to `v` from the next control block.
    fn glide(&mut self, i: usize, v: f32) {
        self.pending[i] = v;
        self.changed = true;
        self.state = 1;
    }
}

/// `JamBurstEchoCore`: three tape units, their filters, the output low pass and the
/// limiter (§3).
struct Core {
    sr: f32,
    lanes: Lanes,
    width: f32,
    synced: bool,
    /// Suspended: parameters land at once.
    direct: bool,
    /// The delay time's one-pole (ms), its input and its coefficient.
    dtime: f64,
    dtime_new: f64,
    dtime_a: f64,
    dt_flag: bool,
    dt_flag2: bool,
    units: [Unit; 3],
    filters: [Filters; 2],
    out: [Svf; 2],
    limiter: Limiter,
}

impl Core {
    fn new(sr: f32) -> Core {
        let mut c = Core {
            sr,
            lanes: Lanes::new(),
            width: 0.0,
            synced: false,
            direct: true,
            dtime: 0.0,
            dtime_new: 0.0,
            dtime_a: 0.0,
            dt_flag: false,
            dt_flag2: false,
            units: [Unit::new(sr), Unit::new(sr), Unit::new(sr)],
            filters: [Filters::new(sr), Filters::new(sr)],
            out: [Svf::default(); 2],
            limiter: Limiter::new(sr),
        };
        c.set_sample_rate(sr);
        c
    }

    fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr;
        let per = BLOCK as i32;
        let steps = ((sr * 0.02) / per as f32 + 0.5) as i32;
        self.lanes.steps = if steps < 2 { 1 } else { steps };
        self.lanes.rate = 1.0 / (self.lanes.steps * per) as f32;
        self.dtime_a = (-5.298_317_366_548_035 / ((sr as f64 * 150.0) / 1000.0)).exp();
        self.units = [Unit::new(sr), Unit::new(sr), Unit::new(sr)];
        let (hp, shelf) = (self.lanes.cur[4], self.lanes.cur[5]);
        self.filters = [Filters::new(sr), Filters::new(sr)];
        for f in &mut self.filters {
            f.set_hp(hp);
            f.set_shelf(shelf);
        }
        self.limiter = Limiter::new(sr);
        self.set_delay_time();
    }

    /// After a parameter while suspended: everything lands, the time's one-pole too.
    fn land(&mut self) {
        self.lanes.jump();
        if self.dt_flag {
            self.dtime = self.dtime_new;
            self.dt_flag = false;
            self.dt_flag2 = true;
        }
    }

    /// `setParameter(index, value)`: 2 high pass (Hz), 3 shelf (Hz), 4 feedback, 5 time
    /// (ms), 6 synced, 7 width.
    fn set(&mut self, index: usize, v: f32) {
        match index {
            2 | 3 => {
                self.lanes.glide(index + 2, v);
                if self.direct {
                    for f in &mut self.filters {
                        if index == 2 {
                            f.set_hp(v);
                        } else {
                            f.set_shelf(v);
                        }
                    }
                }
            }
            4 => {
                if self.direct {
                    self.lanes.put(6, v);
                } else {
                    self.lanes.glide(6, v);
                }
            }
            5 => {
                let v = v.min(30_000.0);
                if self.synced {
                    self.lanes.put(7, v);
                } else {
                    self.lanes.glide(7, v);
                }
            }
            6 => self.synced = v > 0.5,
            7 => self.width = v,
            _ => {}
        }
        if self.direct {
            self.land();
            self.set_delay_time();
        }
    }

    fn set_delay_time(&mut self) {
        let t = if self.dtime < 30_000.0 { self.dtime } else { 30_000.0 };
        let t = if t < 0.1 { 0.1 } else { t };
        let s = 25.0 / t * 0.3;
        let s = if s > 0.3 { 0.3 } else { s };
        let ms = (t as f32) * f32::from_bits(0x3a83_126f);
        self.units[0].set_delay(ms, -(s as f32) as f64);
        self.units[1].set_delay(ms, ((s + s) as f32) as f64);
        self.units[2].set_delay(ms, ((s * -2.0) as f32) as f64);
        for ch in 0..2 {
            let f = self.units[0].len as f32 / self.units[0].delay[ch] as f32;
            let f = if f < f32::from_bits(0x3d4c_cccd) { f32::from_bits(0x3d4c_cccd) } else { f };
            let f = if f > f32::from_bits(0x3f4c_cccd) { f32::from_bits(0x3f75_c290) } else { f * f32::from_bits(0x3f99_999a) };
            let g = tan3(((f as f64 * PI64) * 0.5) as f32);
            self.out[ch].set(g, f32::from_bits(0x3f99_999a));
        }
    }

    fn reset(&mut self) {
        self.out.iter_mut().for_each(Svf::clear);
        self.units.iter_mut().for_each(Unit::reset);
        self.filters.iter_mut().for_each(Filters::reset);
        self.set_delay_time();
        self.land();
    }

    fn resume(&mut self) {
        self.land();
        self.direct = false;
    }

    fn suspend(&mut self) {
        self.direct = true;
        self.land();
    }

    fn frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.lanes.tick();
        let d0 = self.lanes.cur[7] as f64;
        self.dtime_new = d0;
        let d1 = self.dtime;
        if d1 != d0 {
            self.dt_flag = true;
            self.dt_flag2 = false;
        }
        let call = if (d0 - d1).abs() > f64::from_bits(0x3f1a_36e2_eb1c_432d) {
            self.dtime = (1.0 - self.dtime_a).mul_add(d0, d1 * self.dtime_a);
            self.dt_flag || std::mem::take(&mut self.dt_flag2)
        } else if self.dt_flag {
            self.dtime = d0;
            self.dt_flag = false;
            self.dt_flag2 = false;
            true
        } else {
            std::mem::take(&mut self.dt_flag2)
        };
        if call {
            self.set_delay_time();
        }
        self.units[0].write(l, r);
        let (a0l, a0r) = self.units[0].read();
        let (a2l, a2r) = self.units[2].read();
        let fb = self.lanes.cur[6];
        let mut ml = fb * a2l + a0l;
        let mut mr = fb * a2r + a0r;
        if self.lanes.delta[4] != 0.0 {
            let v = self.lanes.cur[4];
            self.filters.iter_mut().for_each(|f| f.set_hp(v));
        }
        if self.lanes.delta[5] != 0.0 {
            let v = self.lanes.cur[5];
            self.filters.iter_mut().for_each(|f| f.set_shelf(v));
        }
        self.filters[0].process(&mut ml, &mut mr);
        self.units[1].write(ml, mr);
        let (mut bl, mut br) = self.units[1].read();
        self.filters[1].process(&mut bl, &mut br);
        self.units[2].write(bl, br);
        let ol = self.out[0].tick(bl + ml).1;
        let or = self.out[1].tick(br + mr).1;
        let m = (or + ol) * 0.5;
        let s = self.width * ((ol - or) * 0.5);
        let s = s + s;
        let (yl, yr) = self.limiter.tick((m + s) * 0.5, (m - s) * 0.5);
        (yl + yl, yr + yr)
    }

    /// `onBPMChanged` with the synced time `ms`.
    fn tempo(&mut self, ms: f32) {
        self.set(5, ms);
    }
}

/// `JamBurstEcho`'s linear 20 ms ramps (§2.2).
#[derive(Clone, Copy, Default)]
struct Smoother {
    cur: f32,
    target: f32,
    step: f32,
    len: f32,
    len_i: i32,
    count: i32,
    active: bool,
}

impl Smoother {
    fn to(&mut self, v: f32) {
        self.target = v;
        if self.cur != v {
            self.step = (v - self.cur) / self.len;
            self.count = self.len_i;
            self.active = true;
        }
    }

    fn land(&mut self) {
        if self.active {
            self.cur = self.target;
            self.step = 0.0;
            self.count = 0;
            self.active = false;
        }
    }

    fn tick(&mut self) -> f32 {
        if (self.target - self.cur).abs() > 0.001 {
            let old = self.count;
            self.count = old - 1;
            if old >= 2 {
                self.cur += self.step;
                return self.cur;
            }
        }
        self.land();
        self.cur
    }
}

/// The feedback for a knob or strip value: v·(1.8 − 0.8|v|)·k (§2.1).
fn feedback(v: f32, k: f32) -> f32 {
    v.abs().mul_add(f32::from_bits(0xbf4c_cccd), f32::from_bits(0x3fe6_6666)) * v * k
}

/// (exponent base, multiplier) of the synced time modes: straight, dotted, triplet.
const SYNC: [(i32, f64); 3] = [(5, 1.0), (25, 1.5), (15, 2.0 / 3.0)];

/// The stored fields in the order Maschine connects them, with their core index.
const FIELDS: [(&str, usize); 8] = [
    ("burst_time", 2),
    ("burst_feedback", 3),
    ("burst_time_mode", 4),
    ("burst_ts_assign", 7),
    ("burst_filter", 5),
    ("burst_feed", 8),
    ("burst_mix_mode", 9),
    ("burst_wet_level", 6),
];

pub struct BurstEcho {
    sr: f32,
    /// The raw parameters by core index: 0 strip, 1 engage, 2 time, 3 feedback, 4 time
    /// mode (÷3), 5 filter, 6 wet, 7 TS assign, 8 feed, 9 mix.
    raw: [f32; 10],
    fb_src: f32,
    strip: f32,
    engaged: bool,
    assign_time: bool,
    burst_len: f32,
    mute_len: f32,
    time_mode: i32,
    time_v: f32,
    sync_idx: i32,
    mix: i32,
    suspended: bool,
    burst_t: f32,
    burst_run: bool,
    mute_t: f32,
    mute_run: bool,
    bpm: f64,
    dt: f64,
    /// Gate, dry, dry mute, wet, replace.
    s: [Smoother; 5],
    core: Core,
    /// Samples into the processor's control block.
    phase: usize,
    current: bool,
    /// To become the current effect at the next frame, with this strip (position, on).
    pending: Option<(f32, bool)>,
    started: bool,
    seen: Vec<(&'static str, f32)>,
}

impl BurstEcho {
    pub fn new(sr: f32) -> BurstEcho {
        let mut b = BurstEcho {
            sr,
            raw: [0.0; 10],
            fb_src: 0.0,
            strip: 0.0,
            engaged: false,
            assign_time: false,
            burst_len: 1.0,
            mute_len: 1.0,
            time_mode: 1,
            time_v: 0.5,
            sync_idx: 5,
            mix: 0,
            suspended: true,
            burst_t: 0.0,
            burst_run: false,
            mute_t: 0.0,
            mute_run: false,
            bpm: 120.0,
            dt: 0.0,
            s: [Smoother::default(); 5],
            core: Core::new(44_100.0),
            phase: 0,
            current: false,
            pending: None,
            started: false,
            seen: Vec::new(),
        };
        b.core.set(7, 0.5);
        b.set_sample_rate(sr);
        b
    }

    /// `onSetSampleRate`: only while suspended and at 44.1 kHz or more; every parameter
    /// is set again.
    fn set_sample_rate(&mut self, sr: f32) {
        if sr < 44_100.0 || !self.suspended {
            return;
        }
        self.sr = sr;
        self.dt = 1.0 / sr as f64;
        let len = sr * 0.001 * 20.0;
        for s in &mut self.s {
            s.len = len;
            s.len_i = len as i32;
            s.count = len as i32;
        }
        self.core.set_sample_rate(sr);
        for i in 0..10 {
            self.set_param(i, self.raw[i]);
        }
    }

    /// The synced time in ms at the tempo, unlimited.
    fn sync_raw(&self) -> f64 {
        let (base, mult) = if (0..3).contains(&self.time_mode) { SYNC[self.time_mode as usize] } else { (0, 0.0) };
        mult * (1000.0 / ((self.bpm / 60.0) * 2f64.powi(base - self.sync_idx)))
    }

    /// The synced time in ms, within 0–30 s.
    fn sync_ms(&self) -> f64 {
        self.sync_raw().clamp(0.0, 30_000.0)
    }

    fn free_ms(&self, v: f32) -> f64 {
        let k = if self.assign_time { 1.0f32 } else { -1.0 };
        let x = (-v.abs()).mul_add(k, k + 1.0) * v;
        let d = x as f64;
        let d = if x < 0.0 {
            0.0
        } else if d > 1.0 {
            1.0
        } else {
            d
        };
        if self.assign_time { d.mul_add(-454.0, 480.0) } else { d.mul_add(760.0, 40.0) }
    }

    /// `setTime`: the echo time for the time knob (or the strip) at `v`.
    fn set_time(&mut self, v: f32) {
        self.time_v = v;
        let ms = if self.time_mode == 3 {
            self.free_ms(v)
        } else {
            let (lo, hi) = match (self.time_mode, self.assign_time) {
                (0, false) => (1, 8),
                (0, true) => (2, 7),
                (1, false) => (21, 28),
                (1, true) => (22, 27),
                (2, false) => (11, 18),
                (2, true) => (12, 17),
                _ => (0, 1),
            };
            let v = if self.assign_time { 1.0 - v } else { v };
            let v = if v > P99 { P99 } else { v };
            self.sync_idx = lo + (v * (hi - lo) as f32) as i32;
            loop {
                if self.sync_raw() <= 30_000.0 {
                    break;
                }
                let old = self.sync_idx;
                self.sync_idx = old - 1;
                if old <= 21 && (1u32 << old) & 0x0020_0802 != 0 {
                    self.sync_idx = 1;
                    break;
                }
            }
            self.sync_ms()
        };
        self.core.set(5, ms as f32);
        self.dry_mute_length();
    }

    /// `calculateDryMuteLength`: the first repeat's time, kept within 70 ms…the burst.
    fn dry_mute_length(&mut self) {
        let ms = if self.time_mode == 3 { self.free_ms(self.time_v) } else { self.sync_ms() };
        let s = ms as f32 * 0.001;
        self.mute_len = if self.burst_len > 0.0 {
            let m = if s < 0.07 { 0.07 } else { s };
            if self.burst_len <= m { self.burst_len } else { m }
        } else {
            s
        };
    }

    fn set_feedback(&mut self, v: f32, k: f32) {
        self.fb_src = v;
        let fb = feedback(v, k);
        self.core.set(4, if !self.engaged && fb > FB_OFF { FB_OFF } else { fb });
    }

    /// `setTouchstripOnOffInternal`.
    fn engage(&mut self, on: bool) {
        if !on {
            self.s[2].to(1.0);
            self.burst_run = false;
            self.s[0].to(0.0);
            self.burst_t = 0.0;
            self.mute_run = false;
            self.mute_t = 0.0;
        } else if !(self.engaged && !self.suspended) {
            self.burst_run = self.burst_len > 0.0;
            self.s[0].to(1.0);
            self.burst_t = 0.0;
            self.mute_run = true;
            self.mute_t = 0.0;
        }
        self.engaged = on;
        let fb = feedback(self.fb_src, if self.assign_time { 1.05 } else { 1.2 });
        self.core.set(4, if !on && fb > FB_OFF { FB_OFF } else { fb });
        self.dry_mute_length();
    }

    fn land_all(&mut self) {
        if self.suspended {
            self.s.iter_mut().for_each(Smoother::land);
        }
    }

    fn set_mix_ramps(&mut self) {
        self.s[4].to(if self.mix == 1 { 1.0 } else { 0.0 });
        self.s[1].to(if self.mix != 2 { 1.0 } else { 0.0 });
    }

    /// `setParameter(index, value)`.
    fn set_param(&mut self, i: usize, v: f32) {
        self.raw[i] = v;
        match i {
            0 => {
                self.strip = v;
                if self.assign_time {
                    self.set_time(v);
                } else {
                    self.set_feedback(v, 1.2);
                }
            }
            1 => self.engage(v > 0.5),
            2 if !self.assign_time => self.set_time(v),
            3 if self.assign_time => self.set_feedback(v, 1.05),
            4 => {
                let x = v * 3.0;
                self.time_mode = (if x >= 0.0 { 0.5 } else { -0.5 } + x) as i32;
                self.core.set(6, if self.time_mode != 3 { 1.0 } else { 0.0 });
                if self.assign_time {
                    self.set_time(self.strip);
                } else {
                    self.set_time(self.raw[2]);
                }
            }
            5 => {
                let hp = exp10f(v.mul_add(f32::from_bits(0x3fcc_cccd), 1.0));
                let c = if v > 1.0 { 1.0 } else { v };
                let c = if v < 0.0 { 0.0 } else { c };
                let shelf = (c as f64).mul_add(-16_000.0, 20_000.0) as f32;
                self.core.set(2, hp);
                self.core.set(3, shelf);
            }
            6 => self.s[3].to(v * 1.5),
            7 => {
                self.assign_time = v > 0.5;
                if self.assign_time {
                    self.fb_src = self.raw[3];
                    let fb = feedback(self.raw[3], 1.05);
                    self.core.set(4, if fb <= FB_OFF || self.engaged { fb } else { FB_OFF });
                }
                self.land_all();
                if !self.assign_time {
                    self.set_time(self.raw[2]);
                }
                self.land_all();
                if self.assign_time {
                    self.set_time(self.strip);
                } else {
                    self.set_feedback(self.strip, 1.2);
                }
                self.dry_mute_length();
            }
            8 => {
                let c = if v > 1.0 { 1.0 } else { v };
                let c = if v < 0.0 { 0.0 } else { c };
                let e = (c as f64).mul_add(25.0, 37.0) as f32;
                let s = (exp10(e as f64 * 0.05) * 0.001) as f32;
                self.burst_len = if v > P99 { 0.0 } else { s };
                if self.engaged {
                    self.burst_run = true;
                    if self.burst_len == 0.0 {
                        self.s[0].to(1.0);
                    }
                }
                self.dry_mute_length();
                self.set_mix_ramps();
            }
            9 => {
                self.mix = (v + 0.5) as i32;
                self.set_mix_ramps();
                self.dry_mute_length();
            }
            _ => {}
        }
        self.land_all();
    }

    fn tempo(&mut self, bpm: f64) {
        self.bpm = bpm;
        if self.time_mode != 3 {
            let ms = self.sync_ms();
            self.core.tempo(ms as f32);
        }
    }

    /// The processor makes this the current effect (`switchEffectMode`): the strip, the
    /// tempo, a reset and a resume.
    fn switch_in(&mut self, pos: f32, on: bool) {
        self.set_param(0, pos);
        self.set_param(1, if on { 1.0 } else { 0.0 });
        self.tempo(self.bpm);
        // onReset.
        self.core.reset();
        self.burst_t = 0.0;
        self.burst_run = false;
        self.mute_t = 0.0;
        self.mute_run = false;
        self.s[2].to(1.0);
        self.s.iter_mut().for_each(Smoother::land);
        let on = self.engaged;
        self.engaged = false;
        self.engage(on);
        // onResume.
        self.core.resume();
        self.suspended = false;
        self.phase = 0;
        self.current = true;
    }

    fn suspend(&mut self) {
        self.core.suspend();
        self.s.iter_mut().for_each(Smoother::land);
        self.suspended = true;
        self.current = false;
    }
}

impl Mode for BurstEcho {
    fn set(&mut self, p: &Params) {
        for (k, i) in FIELDS {
            let v = match i {
                4 => p.i(k) as f32 * f32::from_bits(0x3eaa_aaab),
                7 | 9 => p.i(k) as f32,
                _ => p.f(k),
            };
            match self.seen.iter_mut().find(|(n, _)| *n == k) {
                Some((_, o)) if *o == v => {}
                Some((_, o)) => {
                    *o = v;
                    self.set_param(i, v);
                }
                None => {
                    self.seen.push((k, v));
                    self.set_param(i, v);
                }
            }
        }
        if !self.started {
            self.started = true;
            self.set_sample_rate(self.sr);
        }
        let current = p.i("mode") == 2;
        let pos = p.f("touchstrip_pos");
        let on = p.f("touchstrip_on_off") >= 0.5;
        if !current {
            self.pending = None;
            if self.current {
                self.suspend();
            }
        } else if !self.current {
            // Maschine switches at its next block, with the host's tempo.
            self.pending = Some((pos, on));
        } else {
            if pos != self.raw[0] {
                self.set_param(0, pos);
            }
            if on != (self.raw[1] > 0.5) {
                self.set_param(1, if on { 1.0 } else { 0.0 });
            }
        }
    }

    fn frame(&mut self, l: f32, r: f32, c: &Ctx) -> (f32, f32) {
        if self.pending.is_some() {
            self.phase = 0;
        }
        if self.phase == 0 {
            // The host's tempo is a float in Maschine; recover it from the quarter note.
            let bpm = (60.0 * c.sr as f64 / c.quarter as f64) as f32 as f64;
            if let Some((pos, on)) = self.pending.take() {
                self.bpm = bpm;
                self.switch_in(pos, on);
            } else if bpm != self.bpm {
                self.tempo(bpm);
            }
            self.core.lanes.control();
        }
        self.phase = (self.phase + 1) % BLOCK;
        // The input gate and the burst's timer.
        let g = self.s[0].tick();
        if self.burst_run {
            self.burst_t = (self.dt + self.burst_t as f64) as f32;
            if self.burst_len < self.burst_t {
                self.burst_run = false;
                if self.burst_len != 0.0 {
                    self.s[0].to(0.0);
                }
            }
        }
        let (wl, wr) = self.core.frame(g * l, g * r);
        let dry = self.s[1].tick();
        let mute = self.s[2].tick();
        let wet = self.s[3].tick();
        let rep = self.s[4].tick();
        if self.mute_run {
            self.mute_t = (self.dt + self.mute_t as f64) as f32;
            if self.mute_len < self.mute_t {
                self.s[2].to(0.0);
                self.mute_run = false;
            }
        }
        let d = dry * rep.mul_add(mute - 1.0, 1.0);
        (wet.mul_add(wl, d * l), wet.mul_add(wr, d * r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../out/perform-burst/ref");

    fn params(extra: &[(&str, f64)]) -> Vec<(String, f64)> {
        let mut p: Vec<(String, f64)> = deckard_model::PERFORM_FX.1.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        let mut put = |k: &str, v: f64| match p.iter_mut().find(|(n, _)| n == k) {
            Some(x) => x.1 = v,
            None => p.push((k.to_string(), v)),
        };
        put("mode", 2.0);
        put("touchstrip_on_off", 1.0);
        for (k, v) in extra {
            put(k, *v);
        }
        p
    }

    fn read(path: &str) -> Option<(u32, Vec<f32>, Vec<f32>)> {
        let mut rd = hound::WavReader::open(path).ok()?;
        let spec = rd.spec();
        let v: Vec<f32> = rd.samples::<f32>().map(|s| s.unwrap()).collect();
        let (l, r) = if spec.channels == 2 {
            (v.iter().step_by(2).copied().collect(), v.iter().skip(1).step_by(2).copied().collect())
        } else {
            (v.clone(), v)
        };
        Some((spec.sample_rate, l, r))
    }

    /// Run the Burst Echo as Maschine does in maschine-ref: `pre` samples of silence,
    /// then the input.
    fn run(extra: &[(&str, f64)], sr: f32, pre: usize, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        run_at(extra, sr, 120.0, pre, l, r)
    }

    fn run_at(extra: &[(&str, f64)], sr: f32, bpm: f64, pre: usize, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let p = params(extra);
        let mut fx = BurstEcho::new(sr);
        fx.set(&Params(&p));
        // As mod.rs computes a quarter note.
        let c = Ctx { sr, pos: 0.0, engaged: true, quarter: (60.0 / bpm) as f32 * sr, song: None };
        for _ in 0..pre {
            fx.frame(0.0, 0.0, &c);
        }
        l.iter().zip(r).map(|(&a, &b)| fx.frame(a, b, &c)).unzip()
    }

    /// The residual against Maschine's render in dB (RMS of the difference), and the
    /// first sample that differs.
    fn null(case: &str, bpm: f64, extra: &[(&str, f64)]) -> Option<(f64, Option<usize>)> {
        let (sr, il, ir) = read(&format!("{REF}/{case}-in.wav"))?;
        let (_, ml, mr) = read(&format!("{REF}/{case}.wav"))?;
        let pre = (0.05 * sr as f64) as usize;
        let (dl, dr) = run_at(extra, sr as f32, bpm, pre, &il, &ir);
        let mut e = 0.0f64;
        let mut first = None;
        for i in 0..ml.len() {
            let (a, b) = ((dl[i] - ml[i]) as f64, (dr[i] - mr[i]) as f64);
            if first.is_none() && (a != 0.0 || b != 0.0) {
                first = Some(i);
            }
            e += a * a + b * b;
        }
        let db = 10.0 * (e / (2 * ml.len()) as f64 + 1e-24).log10();
        eprintln!("{case}: residual {db:.1} dB, first difference at {first:?}");
        Some((db, first))
    }

    /// A render: its name, the host tempo and the Perform FX's settings.
    type Case = (&'static str, f64, &'static [(&'static str, f64)]);

    const CASES: [Case; 10] = [
        ("c1", 120.0, &[("touchstrip_pos", 0.5)]),
        ("c2", 120.0, &[("touchstrip_pos", 0.8), ("burst_feed", 1.0), ("burst_mix_mode", 1.0)]),
        (
            "c3",
            120.0,
            &[("touchstrip_pos", 0.4), ("burst_time_mode", 3.0), ("burst_time", 0.3), ("burst_mix_mode", 2.0), ("burst_filter", 0.8)],
        ),
        (
            "c4",
            120.0,
            &[("touchstrip_pos", 0.3), ("burst_ts_assign", 1.0), ("burst_feedback", 0.6), ("burst_time_mode", 2.0), ("burst_feed", 0.7)],
        ),
        ("c5", 120.0, &[("touchstrip_pos", 0.95), ("burst_feed", 1.0), ("burst_feedback", 1.0), ("burst_filter", 0.0)]),
        ("off", 120.0, &[("touchstrip_on_off", 0.0)]),
        ("c6", 90.0, &[("touchstrip_pos", 0.7), ("burst_ts_assign", 1.0), ("burst_time_mode", 2.0), ("burst_feed", 1.0)]),
        (
            "c7",
            120.0,
            &[
                ("touchstrip_pos", 0.2),
                ("burst_ts_assign", 1.0),
                ("burst_time_mode", 3.0),
                ("burst_mix_mode", 1.0),
                ("burst_feed", 0.5),
                ("burst_feedback", 0.9),
            ],
        ),
        ("c8", 120.0, &[("touchstrip_pos", 1.0), ("burst_feed", 1.0), ("burst_wet_level", 1.0), ("burst_filter", 0.1)]),
        (
            "c10",
            137.5,
            &[("touchstrip_pos", 0.6), ("burst_time_mode", 0.0), ("burst_time", 0.05), ("burst_feed", 1.0), ("burst_mix_mode", 1.0)],
        ),
    ];

    /// Null tests against Maschine 3's own renders (out/perform-burst/ref, made with
    /// maschine-ref; skipped when absent).
    #[test]
    fn nulls_against_maschine() {
        let mut worst = -1000.0f64;
        for (case, bpm, extra) in CASES {
            let Some((db, _)) = null(case, bpm, extra) else { return };
            worst = worst.max(db);
        }
        assert!(worst < -120.0, "worst residual {worst:.1} dB");
    }

    /// Above 48 kHz Maschine runs the core at a divided rate through sample-rate
    /// converters (not reproduced): measured, not asserted.
    #[test]
    fn residual_at_96k() {
        null("c9", 120.0, &[("touchstrip_pos", 0.5), ("burst_feed", 1.0)]);
    }

    /// Engaged on a click, it repeats the click a dotted eighth later (Maschine's default
    /// time at 120 BPM: 375 ms, give or take the tape's stereo spread), and the burst
    /// closes the input after it.
    #[test]
    fn echoes_the_burst() {
        let sr = 48_000.0;
        let pre = 48_000;
        let mut x = vec![0.0f32; 96_000];
        x[0] = 0.5;
        let (l, _) = run(&[("touchstrip_pos", 0.5), ("burst_mix_mode", 2.0), ("burst_feed", 1.0)], sr, pre, &x, &x);
        let peak = l.iter().enumerate().skip(100).max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert!((17_000..19_500).contains(&peak), "first repeat at {peak}");
    }
}
