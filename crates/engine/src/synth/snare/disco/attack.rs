//! `NI::DSP::AttackCore<FluffyOsc on, white noise, sine, no saturator, no anticlick,
//! phase coupling on, pitch envelope on, sine phase oscillators>` at `DiscoSDCore`
//! +0x100 (offsets below are relative to the AttackCore, A).
//!
//! A note first plays the "fluffy" one-shot (a shaped half-ramp); when it ends, the
//! main oscillator (a sine with a pitch envelope, phase-modulated by four phase
//! oscillators, through a one-pole lowpass) and a filtered noise burst start. The
//! three are mixed with fixed gains.

use super::kit::{MemoEnv, e, e_rate, et, vcurve2};
use super::vcurve;

/// The parabolic sine of NI's oscillators for a phase `p` (cycles), as compiled:
/// wrap to [−0.5, 0.5] (round half away), fold, `s·(π + s²(−5.13274 + 2.26548 s²))`.
#[inline]
fn sine(p: f32) -> f32 {
    let p = p - round_half(p) as f32;
    let p = (p + p).abs();
    let s = 0.5 - p;
    let q = s * s;
    let a = q.mul_add(f32::from_bits(0x4010_fda0), f32::from_bits(0xc0a4_3f68));
    s * a.mul_add(q, f32::from_bits(0x4049_0fd0))
}

/// `fcvtzs(x as f64 ± 0.5)`.
#[inline]
fn round_half(x: f32) -> i32 {
    let h = if x < 0.0 { -0.5f64 } else { 0.5f64 };
    (x as f64 + h) as i32
}

/// `NI::DSP::UTIL::PhaseOsc<sine>` (0xc0 bytes).
#[derive(Clone, Copy)]
struct PhaseOsc {
    /// +0 frequency (Hz), +4 release, +8 level, +0xc on, +0xe phase reset at a note.
    freq: f32,
    level: f32,
    on: bool,
    reset: bool,
    env: MemoEnv,
    /// +0x98 phase, +0x9c last sine, +0xa4 1/sr, +0xa8 (stored), +0xac random
    /// detune, +0xb0 level offset, +0xb4 gain, +0xb8 phase offset (coupling input),
    /// +0xbc frequency ratio.
    phase: f32,
    out: f32,
    inv: f32,
    aa: f32,
    fm: f32,
    lvl_add: f32,
    gain: f32,
    ph_in: f32,
    ratio: f32,
}

impl PhaseOsc {
    fn new(freq: f32, rel: f32, level: f32, on: bool, gain: f32) -> PhaseOsc {
        PhaseOsc {
            freq,
            level,
            on,
            reset: true,
            env: MemoEnv::new(-24.0, 0.0, 0.0, rel),
            phase: 0.0,
            out: 0.0,
            inv: 0.0,
            aa: 0.0,
            fm: 0.0,
            lvl_add: 0.0,
            gain,
            ph_in: 0.0,
            ratio: 1.0,
        }
    }

    /// `init(sr, attack, _, reset)`.
    fn init(&mut self, sr: f32, att: f32, reset: bool) {
        self.inv = 1.0 / sr;
        self.aa = 0.0;
        let env = &mut self.env;
        env.set_sample_rate(sr);
        env.hold_len = (env.sr * (et(602, f32::from_bits(0x3d58_c000)) + -0.001)) as f64;
        env.h = 0.0;
        env.d = 0.0;
        env.dec_rate = env.inv * et(1797, f32::from_bits(0x3f72_7000)) as f64;
        env.sus = 1.0;
        env.a = att;
        env.att_rate = e_rate(att) as f64 * env.inv;
        self.reset = reset;
        env.rel = env.rel_for(env.r);
    }

    fn process(&mut self) -> f32 {
        if !self.on {
            return 0.0;
        }
        let inc = ((self.fm + 1.0) * self.freq) * self.ratio * self.inv;
        let inc = if inc < -1.0 { -1.0 } else { inc };
        let inc = if inc > 1.0 { 1.0 } else { inc };
        let p = self.phase + inc;
        let p = if p < 0.5 { p } else { p + -1.0 };
        let a = if inc < 0.01 { 0.01 } else { inc };
        let a = if a > 0.03 { 0.02 } else { a + -0.01 };
        self.aa = inc * 100.0 * a;
        let t = self.ph_in + p;
        let t = t - (t as i32) as f32;
        let s = sine(t + -0.25);
        self.phase = p;
        self.out = s;
        let lvl = self.level + self.lvl_add;
        let lvl = if lvl < 0.0 { 0.0 } else { lvl };
        let lvl = if lvl > 1.0 { 1.0 } else { lvl };
        (self.gain * lvl * s) * self.env.process()
    }
}

/// `NI::DSP::UTIL::MainOscAttack<sine, …>` (A+8; offsets relative to it).
struct MainOsc {
    /// +0 frequency (Hz), +4 shape (−1..1), +8 phase reset, +0xc random detune depth,
    /// +0x10 the phase oscillators' attack, +0x14/+0x18 pitch-envelope depths.
    freq: f32,
    shape: f32,
    reset: bool,
    detune: f32,
    osc_att: f32,
    pe1: f32,
    pe2: f32,
    /// +0x1c..+0x28: envelope 3's velocity curve (lo, hi, k, m).
    e3_curve: [f32; 4],
    /// +0x54 coupling depth (osc 1 → osc 2 phase), +0x58/+0x5c frequency factors
    /// (+0x5c: `setModulationFreqPhOscRatio`).
    couple: f32,
    fmul1: f32,
    fmul2: f32,
    /// +0x60/+0x64 lowpass cutoff range (pitch units), +0x68..+0x74 its velocity
    /// curve (lo, hi, k1, k2), +0x78..+0x84 the envelopes' velocity curve.
    cut: [f32; 2],
    cut_curve: [f32; 4],
    env_curve: [f32; 4],
    /// +0x88 amplitude, +0x108 pitch, +0x188 third (unused) envelope.
    env1: MemoEnv,
    env2: MemoEnv,
    env3: MemoEnv,
    /// +0x218 lowpass state, +0x21c highpass state, +0x228 coefficient, +0x22c cutoff
    /// (Hz), +0x230 1/sr.
    lp: f32,
    hp: f32,
    g: f32,
    fc: f32,
    lp_inv: f32,
    osc: [PhaseOsc; 4],
    /// +0x538 velocity, +0x54c phase, +0x554 1/sr, +0x564 seed, +0x56c last cutoff.
    vel: f32,
    phase: f32,
    inv: f32,
    seed: i32,
    cut_last: f32,
}

impl MainOsc {
    fn new() -> MainOsc {
        MainOsc {
            freq: 200.0,
            shape: 0.0,
            reset: true,
            detune: 0.0,
            osc_att: -24.0,
            pe1: 0.48,
            pe2: 0.0,
            e3_curve: [0.0, 1.0, 0.0, 0.0],
            couple: 0.92,
            fmul1: 0.842_5,
            fmul2: 1.0,
            cut: [0.0, 135.0],
            cut_curve: [0.0, 0.85, 1.0, -1.0],
            env_curve: [0.15, 1.0, -0.824, -0.1],
            env1: MemoEnv::new(0.0, 6.88, 0.0, 28.0),
            env2: MemoEnv::new(3.0, 0.0, 0.0, 17.0),
            env3: MemoEnv::new(0.0, 0.0, 0.0, 0.0),
            lp: 0.0,
            hp: 0.0,
            g: 0.0,
            fc: -1.0,
            lp_inv: 0.0,
            osc: [
                PhaseOsc::new(408.0, 2.0, 0.186, true, 1.0),
                PhaseOsc::new(900.0, 10.0, 0.06, true, -1.0),
                PhaseOsc::new(1644.0, 7.0, 0.098, true, 1.0),
                PhaseOsc::new(0.0, 0.0, 0.0, false, 1.0),
            ],
            vel: 0.0,
            phase: 1.0,
            inv: 0.0,
            seed: 1,
            cut_last: -1.0,
        }
    }

    /// `setSampleRate`.
    fn set_sample_rate(&mut self, sr: f32) {
        let inv = 1.0 / sr;
        self.inv = inv;
        self.phase = 1.0;
        for o in self.osc.iter_mut() {
            o.init(sr, self.osc_att, self.reset);
        }
        for env in [&mut self.env1, &mut self.env2, &mut self.env3] {
            env.set_sample_rate(sr);
            env.d = 0.0;
            env.dec_rate = env.inv * et(1797, f32::from_bits(0x3f72_7000)) as f64;
            env.sus = 1.0;
        }
        self.lp_inv = inv;
        let f = self.fc;
        self.fc = -1.0;
        if f != -1.0 {
            self.fc = f;
            let g = (inv * std::f32::consts::TAU) * f;
            self.g = if g > 1.0 { 1.0 } else { g };
        }
        self.lp = 0.0;
        self.hp = 0.0;
        self.shape = self.shape.clamp(-1.0, 1.0);
        let rate = e_rate(self.osc_att) as f64;
        for o in self.osc.iter_mut() {
            o.env.a = self.osc_att;
            o.env.att_rate = rate * o.env.inv;
        }
    }

    /// `setNoteOn(velocity)`.
    fn note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        self.vel = v;
        let [lo, hi, k1, k2] = self.env_curve;
        let x = vcurve2(v, lo, hi, k1, k2);
        self.env1.note_on(x);
        self.env2.note_on(x);
        let [lo, hi, k, m] = self.e3_curve;
        self.env3.note_on(vcurve(x, lo, hi, k, m));
        self.seed = self.seed.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        let rnd = (self.detune * self.detune) * (self.seed as f32 * f32::from_bits(0x3000_009d));
        for o in self.osc.iter_mut() {
            o.fm = rnd;
            o.env.note_on(x);
            if o.reset {
                o.phase = 0.0;
            }
        }
        if self.reset {
            self.phase = 1.0;
        }
    }

    fn process(&mut self) -> f32 {
        let f = (self.freq * self.fmul1) * self.fmul2;
        let pe = self.pe1 + self.pe2;
        let pe = if pe < 0.0 { 0.0 } else { pe };
        let pe = if pe > 5.0 { 5.0 } else { pe };
        let env = self.env2.process();
        let inc = self.inv * (f * pe.mul_add(env, 1.0));
        let inc = if inc < -1.0 { -1.0 } else { inc };
        let inc = if inc > 1.0 { 1.0 } else { inc };
        let p = self.phase + inc;
        let p = if p < 0.5 { p } else { p + -1.0 };
        self.phase = p;
        let couple = self.couple;
        let mut m = self.osc[0].process() + 0.0;
        if couple > 0.0 {
            self.osc[1].ph_in = self.osc[0].out * self.couple;
        }
        m += self.osc[1].process();
        m += self.osc[2].process();
        let x = p + (m + self.osc[3].process());
        let x = x - round_half(x) as f32;
        let x = x + x;
        let k = self.shape;
        let c = 1.0 - k;
        let x = x * x.abs().mul_add(k, c);
        let x = x * x.abs().mul_add(k, c);
        let s = sine(x.mul_add(0.5, -0.25));
        let amp = self.env1.process();
        let [lo, hi, k1, k2] = self.cut_curve;
        let y = vcurve2(self.vel, lo, hi, k1, k2);
        let cut = (self.cut[1] - self.cut[0]).mul_add(y, self.cut[0]);
        if cut != self.cut_last {
            let fc = e(cut.mul_add(5.0, f32::from_bits(0x44ac_bc36)));
            if self.fc != fc {
                self.fc = fc;
                let g = fc * (self.lp_inv * std::f32::consts::TAU);
                self.g = if g > 1.0 { 1.0 } else { g };
            }
            self.cut_last = cut;
        }
        let lp = self.g.mul_add(self.hp, self.lp);
        let hp = s * amp - lp;
        self.lp = lp;
        self.hp = hp;
        lp
    }
}

/// `NI::DSP::UTIL::FluffyOsc` (A+0x578): a one-shot ramp from −1 to `lim`, shaped.
struct Fluffy {
    /// +0..+0xc velocity curve (lo, hi, k, m), +0x10 mode (1: half period),
    /// +0x14 end value, +0x18 level, +0x1c frequency, +0x20 shape.
    curve: [f32; 4],
    mode: f32,
    lim: f32,
    level: f32,
    freq: f32,
    shape: f32,
    /// +0x24 velocity gain, +0x28 value, +0x34 step, +0x38 on, +0x3c count,
    /// +0x40 length.
    vel: f32,
    value: f32,
    inc: f32,
    on: bool,
    n: u32,
    len: u32,
}

impl Fluffy {
    fn set_sample_rate(&mut self, sr: f32) {
        let inv = 1.0 / sr;
        let s = (inv + inv) * self.freq;
        let s = if s < -1.0 { -1.0 } else { s };
        self.inc = if s > 1.0 { 1.0 } else { s };
        let m = if self.mode == 1.0 { 2.0 } else { 1.0 };
        let l = (1.0 / (self.freq * m)) * sr;
        let l = if l < 0.0 { l as f64 } else { l as f64 + 1.0 };
        self.len = l as i32 as u32;
        self.vel = 0.0;
    }

    fn note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        self.value = -1.0;
        self.n = 0;
        self.on = true;
        let [lo, hi, k, m] = self.curve;
        self.vel = vcurve(v, lo, hi, k, m);
    }

    fn process(&mut self) -> f32 {
        let v = self.value + self.inc;
        let v = if self.lim < v { self.lim } else { v };
        self.value = v;
        if self.n >= self.len {
            self.on = false;
        }
        let k = self.shape;
        let c = k + 1.0;
        let y = v * (-v.abs()).mul_add(k, c);
        let y = (y * (-y.abs()).mul_add(k, c)) * 0.5;
        let out = (self.vel * self.level) * (y * y.abs().mul_add(-16.0, 8.0));
        self.n = self.n.wrapping_add(1);
        out
    }
}

/// `ParallelNoise`'s first filter: a TPT state-variable filter, highpass output.
#[derive(Default)]
struct Svf3 {
    /// +0x198.. states, +0x1b0 g, +0x1b4 k, +0x1b8 g(g + k), +0x1bc cutoff,
    /// +0x1c0 1/(g(g + k) + 1), +0x1c4 "reset k", +0x1cc 1/sr.
    z: [f32; 3],
    g: f32,
    k: f32,
    gk: f32,
    fc: f32,
    h: f32,
    reset_k: bool,
    inv: f32,
}

impl Svf3 {
    /// Cutoff `f` (stored clamped to [0, 0.49·sr]; `g` uses the unclamped value):
    /// g = w + w³/3 + 2w⁵/15 (tan), w = π·f/sr.
    fn set_freq(&mut self, f: f32, sr: f32) {
        let lim = sr * f32::from_bits(0x3efa_e148);
        let c = if f < 0.0 { 0.0 } else { f };
        self.fc = if c > lim { lim } else { c };
        let w = self.inv * (f * std::f32::consts::PI);
        let w2 = w * w;
        let w3 = w * w2;
        let w5 = w2 * w3;
        let t = w5.mul_add(f32::from_bits(0x3e08_8886), w3 * f32::from_bits(0x3eaa_aaaa));
        self.g = w + t;
        self.gk = self.g * (self.g + self.k);
        self.h = 1.0 / (self.gk + 1.0);
    }
}

/// `NI::DSP::UTIL::ParallelNoise<white>` (A+0x5c0; offsets relative to it).
struct Noise {
    /// +0 level, +8 highpass cutoff, +0x10 lowpass cutoff (Hz).
    level: f32,
    hp_fc: f32,
    lp_fc: f32,
    /// +0x30..+0x3c velocity curve (lo, hi, k1, k2).
    curve: [f32; 4],
    /// The NoiseGen (+0x54): +0x5c initial seed, +0xe4 scale, +0xec seed.
    seed0: i32,
    scale: f32,
    seed: i32,
    /// +0x118.
    env: MemoEnv,
    svf: Svf3,
    /// The second filter (+0x1d0..): previous input, two states, g, k, coefficient,
    /// cutoff, "reset k", 1/sr.
    prev: f32,
    z1: f32,
    z2: f32,
    g2: f32,
    k2: f32,
    c2: f32,
    fc2: f32,
    reset_k2: bool,
    inv2: f32,
}

impl Noise {
    fn new() -> Noise {
        Noise {
            level: f32::from_bits(0x3ec2_480e),
            hp_fc: f32::from_bits(0x4412_d560),
            lp_fc: f32::from_bits(0x45ae_9d87),
            curve: [0.0, 1.0, -1.0, f32::from_bits(0xbdac_0831)],
            seed0: 2000,
            scale: 0.0,
            seed: 2000,
            env: MemoEnv::new(-0.5, 0.0, -5.0, 28.45),
            svf: Svf3 { k: 2.0, fc: f32::from_bits(0x4412_d560), ..Default::default() },
            prev: 0.0,
            z1: 0.0,
            z2: 0.0,
            g2: 0.0,
            k2: 2.0,
            c2: 0.0,
            fc2: f32::from_bits(0x45ae_9d87),
            reset_k2: false,
            inv2: 0.0,
        }
    }

    /// The second filter's coefficients for cutoff `f`.
    fn set_lp(&mut self, f: f32) {
        self.fc2 = f;
        let g = ((f * std::f32::consts::TAU) * self.inv2).min(f32::from_bits(0x3ff3_3333));
        self.g2 = g;
        let a = ((g + 2.0) * (2.0 - g)) * self.k2;
        let b = (g.mul_add(self.k2, 2.0) - g) * 2.0;
        self.c2 = a / b;
    }

    fn set_sample_rate(&mut self, sr: f32) {
        let inv = 1.0 / sr;
        self.env.set_sample_rate(sr);
        self.svf.inv = inv;
        let old = self.svf.fc;
        self.svf.fc = -1.0;
        if old != -1.0 {
            self.svf.set_freq(old, sr);
        }
        if self.svf.fc != self.hp_fc {
            self.svf.set_freq(self.hp_fc, sr);
        }
        if self.svf.reset_k {
            self.svf.reset_k = false;
            self.svf.k = 2.0;
            self.svf.gk = self.svf.g * (self.svf.g + 2.0);
            self.svf.h = 1.0 / (self.svf.gk + 1.0);
        }
        self.inv2 = inv;
        let old = self.fc2;
        self.fc2 = -1.0;
        if old != -1.0 {
            self.set_lp(old);
        }
        if self.fc2 != self.lp_fc {
            self.set_lp(self.lp_fc);
        }
        if self.reset_k2 {
            self.reset_k2 = false;
            self.k2 = 2.0;
            let g = self.g2;
            self.c2 = (((g + 2.0) * (2.0 - g)) * 2.0) / ((g.mul_add(2.0, 2.0) - g) * 2.0);
        }
        // NoiseGen<white>::setSampleRate.
        self.scale = (sr / 44_100.0).sqrt() * f32::from_bits(0x3000_009d);
    }

    fn note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        let [lo, hi, k1, k2] = self.curve;
        let x = vcurve2(v, lo, hi, k1, k2);
        if x > 0.0 {
            // NoiseGen::setNoteOn would reseed, but its flag (+0x54) is off here.
            self.env.note_on(x);
        }
    }

    fn process(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
        let x = self.level * (self.scale * self.seed as f32);
        let f = &mut self.svf;
        let [z1, z2, z3] = f.z;
        let t = z2 * f.g.mul_add(2.0, f.k);
        let t = z3 + z1.mul_add(f.gk, t);
        let hp = f.h * (x - t);
        let bp = (z1 + hp).mul_add(f.g, z2);
        let lp = (z2 + bp).mul_add(f.g, z3);
        f.z = [hp, bp, lp];
        let x = (self.prev + hp) * 0.25;
        self.prev = hp;
        let s = x - self.c2.mul_add(self.z1, self.z2);
        let b = self.g2.mul_add(s, self.z1);
        let l = self.g2.mul_add(b, self.z2);
        let out = self.z2 + l;
        self.z1 = b;
        self.z2 = l;
        out * self.env.process()
    }
}

/// The attack sub-core.
pub(super) struct Attack {
    main: MainOsc,
    fluffy: Fluffy,
    noise: Noise,
    /// +0x7ec/+0x7f4/+0x7fc: fluffy, main and noise gains (−4, −1, −9.5 dB; their
    /// smoother at +0x840 never moves).
    gains: [f32; 3],
    /// +0x7e0 sample rate, +0x80c the attack length (ms) from the delay parameters
    /// +0x818..+0x820, +0x824 the attack envelope decay.
    sr: f32,
    delay: f32,
    delay_p: [f32; 3],
    env_decay: f32,
    /// +0x810 stored velocity, +0x814 fluffy started (the rest follows), +0x828 busy.
    vel: f32,
    second: bool,
    busy: bool,
}

impl Attack {
    pub(super) fn new() -> Attack {
        Attack {
            main: MainOsc::new(),
            fluffy: Fluffy {
                curve: [0.72, 1.0, 0.26, 1.0],
                mode: 1.0,
                lim: 0.0,
                level: 1.0,
                freq: 671.7,
                shape: -0.57,
                vel: 0.0,
                value: -1.0,
                inc: 0.0,
                on: false,
                n: 0,
                len: 0,
            },
            noise: Noise::new(),
            gains: [et(1160, 0.136_840_82), et(1190, 0.034_179_688), et(1105, 0.325_073_24)],
            sr: 0.0,
            delay: 0.0,
            delay_p: [0.0, 6.88, 0.0],
            env_decay: 28.0,
            vel: 0.0,
            second: false,
            busy: false,
        }
    }

    pub(super) fn set_sample_rate(&mut self, sr: f32, _block: i32, _steps: i32, _inv: f32) {
        self.sr = sr;
        self.main.set_sample_rate(sr);
        self.fluffy.set_sample_rate(sr);
        self.noise.set_sample_rate(sr);
        self.update_delays();
    }

    /// `updateDelays`: the fluffy period plus two times (ms).
    fn update_delays(&mut self) {
        let m = if self.fluffy.mode == 1.0 { 2.0 } else { 1.0 };
        let p = 1.0 / (self.fluffy.freq * m);
        let a = e(self.delay_p[0].mul_add(9.965_784, 602.052_9));
        let b = e((self.delay_p[1] + self.delay_p[2]).mul_add(9.965_784, 602.052_9));
        self.delay = (p + (a + b)) * 1000.0;
    }

    pub(super) fn reset(&mut self) {
        self.second = false;
        self.busy = false;
        let m = &mut self.main;
        m.phase = 1.0;
        m.seed = 1;
        for env in [&mut m.env1, &mut m.env2, &mut m.env3] {
            env.state = 0;
            env.value = 0.0;
        }
        for o in m.osc.iter_mut() {
            o.phase = 0.0;
            o.aa = 0.0;
        }
        m.lp = 0.0;
        m.hp = 0.0;
        m.cut_last = -1.0;
        let f = &mut self.fluffy;
        f.value = -1.0;
        f.on = false;
        f.n = 0;
        let n = &mut self.noise;
        n.seed = n.seed0;
        n.env.state = 0;
        n.env.value = 0.0;
        n.svf.z = [0.0; 3];
        n.prev = 0.0;
        n.z1 = 0.0;
        n.z2 = 0.0;
    }

    /// `AttackCore::setNoteOn`: the fluffy one-shot first; the oscillator and the
    /// noise when it has ended (or at the next note).
    pub(super) fn note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        self.busy = true;
        if self.second {
            self.main.note_on(v);
            self.noise.note_on(v);
            self.second = false;
        } else {
            self.vel = v;
            self.fluffy.note_on(v);
            self.second = true;
        }
    }

    pub(super) fn process(&mut self) -> f32 {
        let fl = if self.fluffy.on {
            self.fluffy.process()
        } else {
            if self.second {
                self.note_on(self.vel);
            }
            0.0
        };
        let m = self.main.process();
        let n = self.noise.process();
        if !self.fluffy.on && self.main.env1.state == 0 && self.noise.env.state == 0 {
            self.busy = false;
        }
        let [g0, g1, g2] = self.gains;
        g2.mul_add(n, g0.mul_add(fl, m * g1))
    }

    /// The attack gains' smoother (inline in `DiscoSDCore::processControlRate`): its
    /// targets never change, so it never moves.
    pub(super) fn control_rate(&mut self) {}

    pub(super) fn active(&self) -> bool {
        self.fluffy.on || self.main.env1.state != 0 || self.noise.env.state != 0 || self.busy
    }

    pub(super) fn delay_ms(&self) -> f32 {
        self.delay
    }

    pub(super) fn env_decay_ms(&self) -> f32 {
        self.env_decay
    }

    /// `setModulationNoiseLP(x)`: the noise lowpass at E(5·(113 + x) + 1381.88) Hz,
    /// clamped to [20, 20000].
    pub(super) fn set_noise_lp(&mut self, v: f32) {
        if !(-60.0..=60.0).contains(&v) {
            return;
        }
        let f = e((113.0 + v).mul_add(5.0, f32::from_bits(0x44ac_bc36)));
        let f = if f < 20.0 { 20.0 } else { f };
        let f = if f > 20_000.0 { 20_000.0 } else { f };
        self.noise.lp_fc = f;
        if self.noise.fc2 != f {
            self.noise.set_lp(f);
        }
    }

    /// `setModulationPNoiseEnvDec2(x)`: the noise envelope's release = 28.45 + x.
    pub(super) fn set_pnoise_env_dec2(&mut self, v: f32) {
        if !(-28.0..=40.0).contains(&v) {
            return;
        }
        let env = &mut self.noise.env;
        env.r = 28.45 + v;
        env.rel = env.rel_for(env.r);
    }

    /// `setModulationFreqPhOscRatio(x)`: oscillator frequency ratio E(5x + 1200).
    pub(super) fn set_freq_ph_osc_ratio(&mut self, v: f32) {
        if !(-64.0..=64.0).contains(&v) {
            return;
        }
        let f = e(v.mul_add(5.0, 1200.0)).max(0.0);
        self.main.fmul2 = f;
        for o in self.main.osc.iter_mut() {
            o.ratio = f;
        }
    }
}

#[cfg(test)]
impl Attack {
    pub(super) fn dbg(&self) -> String {
        let m = &self.main;
        format!(
            "fmul2 {} g {} fc {} vel {} cut_last {} phase {} lp {} hp {} env1 st{} v{} peak{} inc{} env2 st{} v{} | osc1 ph {} out {} env st{} v{}",
            m.fmul2,
            m.g,
            m.fc,
            m.vel,
            m.cut_last,
            m.phase,
            m.lp,
            m.hp,
            m.env1.state,
            m.env1.value,
            m.env1.peak,
            m.env1.att_inc,
            m.env2.state,
            m.env2.value,
            m.osc[0].phase,
            m.osc[0].out,
            m.osc[0].env.state,
            m.osc[0].env.value
        ) + &format!(
            " | noise seed {} z {:?} prev {} z1 {} z2 {} g2 {} c2 {} g {} gk {} h {} scale {} env st{} v{} inc{} gains {:?}",
            self.noise.seed,
            self.noise.svf.z,
            self.noise.prev,
            self.noise.z1,
            self.noise.z2,
            self.noise.g2,
            self.noise.c2,
            self.noise.svf.g,
            self.noise.svf.gk,
            self.noise.svf.h,
            self.noise.scale,
            self.noise.env.state,
            self.noise.env.value,
            self.noise.env.att_inc,
            self.gains
        )
    }
}
