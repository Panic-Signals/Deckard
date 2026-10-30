//! The building blocks `NI::DSP::ClapCore` inlines (docs/synth-snare.md, Pow/Clap):
//! the drum-synth `tan` approximation, the TPT one-pole, the "SVF" state-variable
//! filter, `UTIL::BiQuad`, the retrigger de-click "slew" stage, the clap hit envelope
//! and `UTIL::FreeEnvelope<off, breakpoint, shape, shape|log, lin>`. Every operation
//! follows the assembly; `mul_add` stands exactly where it has fmadd/fmsub.

const fn f(bits: u32) -> f32 {
    f32::from_bits(bits)
}

pub const PI: f32 = f(0x4049_0fdb);
const THIRD: f32 = f(0x3eaa_aaaa);
const TWO_15: f32 = f(0x3e08_8886);
/// `1e-4`, the threshold of the slews and the hit envelope.
pub const THR: f32 = f(0x38d1_b717);

/// `x + x³/3 + 2x⁵/15` (the drum synths' `tan(x)`).
#[inline]
pub fn tan_w(w: f32) -> f32 {
    let w2 = w * w;
    let w3 = w * w2;
    let w5 = w2 * w3;
    let a = w3 * THIRD;
    w + w5.mul_add(TWO_15, a)
}

/// `2^x` as the cores compute it: round to nearest (`frinti`), a cubic or quadratic
/// polynomial for the fraction, the exponent added to the bits of 1.0.
#[inline]
fn pow2_parts(x: f32) -> (f32, f32) {
    let i = x.round_ties_even();
    let n = if i < 2_147_483_648.0 && i > -2_147_483_648.0 { i as i32 } else { i32::MIN };
    let e = f32::from_bits(0x3f80_0000u32.wrapping_add((n as u32).wrapping_shl(23)));
    (x - n as f32, e)
}

/// The slews' `2^x` (cubic).
#[inline]
pub fn pow2_slew(x: f32) -> f32 {
    let (fr, e) = pow2_parts(x);
    let p = fr.mul_add(f(0x3d65_6042), f(0x3e77_ced9));
    let p = p.mul_add(fr, f(0x3f31_6873));
    p.mul_add(fr, 1.0) * e
}

/// BiQuad's `2^x` (quadratic).
#[inline]
fn pow2_bq(x: f32) -> f32 {
    let (fr, e) = pow2_parts(x);
    let p = fr.mul_add(f(0x3e7d_f3b6), f(0x3f35_3f7d));
    p.mul_add(fr, 1.0) * e
}

/// A TPT one-pole (`g = tan(π·fc/sr)`); the coefficients follow `fc` when it changes.
#[derive(Clone, Copy)]
pub struct OnePole {
    pub z: f32,
    pub out: f32,
    /// `1/(g+1)`, `g − 1`, `g`, cutoff, `1/sr`.
    pub a: f32,
    pub b: f32,
    pub g: f32,
    pub fc: f32,
    pub inv_sr: f32,
}

impl OnePole {
    pub fn new(fc: f32) -> OnePole {
        OnePole { z: 0.0, out: 0.0, a: 1.0, b: 0.0, g: 1.0, fc, inv_sr: 1.0 }
    }

    pub fn coef(&mut self, fc: f32) {
        self.fc = fc;
        let g = tan_w(fc * PI * self.inv_sr);
        self.g = g;
        self.a = 1.0 / (g + 1.0);
        self.b = g + -1.0;
    }

    /// Recompute only when `fc` differs from the stored cutoff.
    #[inline]
    pub fn follow(&mut self, fc: f32) {
        if self.fc != fc {
            self.coef(fc);
        }
    }

    /// Low-pass output.
    #[inline]
    pub fn lp(&mut self, x: f32) -> f32 {
        let y = self.a * (-self.b).mul_add(self.z, x);
        let o = self.g * (self.z + y);
        self.out = o;
        self.z = y;
        o
    }

    /// High-pass output (`y − z`).
    #[inline]
    pub fn hp(&mut self, x: f32) -> f32 {
        let y = self.a * (-self.b).mul_add(self.z, x);
        let o = y - self.z;
        self.out = o;
        self.z = y;
        o
    }
}

/// The state-variable filter (`k` damping, `h = 1/(g(g+k)+1)`), band-pass and low-pass
/// outputs.
#[derive(Clone, Copy)]
pub struct Svf {
    pub s1: f32,
    pub s2: f32,
    pub bp: f32,
    pub lp: f32,
    pub g: f32,
    pub k: f32,
    pub fc: f32,
    /// The resonance `r` (`k = 2(1 − r)`).
    pub res: f32,
    pub h: f32,
    pub sr: f32,
    pub inv_sr: f32,
}

impl Svf {
    pub fn new(k: f32, res: f32) -> Svf {
        Svf { s1: 0.0, s2: 0.0, bp: 0.0, lp: 0.0, g: 0.0, k, fc: 1.0, res, h: 1.0, sr: 1.0, inv_sr: 1.0 }
    }

    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
        self.bp = 0.0;
        self.lp = 0.0;
    }

    /// The per-sample coefficient check (stored cutoff is the clamped one).
    #[inline]
    pub fn follow(&mut self, fc: f32) {
        if self.fc == fc {
            return;
        }
        let lim = self.sr * f(0x3efa_e148);
        let c = if fc < 0.0 { 0.0 } else { fc };
        let c = if c > lim { lim } else { c };
        self.fc = c;
        self.g = tan_w(self.inv_sr * (c * PI));
        self.h = 1.0 / self.g.mul_add(self.k + self.g, 1.0);
    }

    /// `setResonance` (at a mode change).
    pub fn set_res(&mut self, r: f32) {
        if self.res != r {
            self.res = r;
            self.k = (1.0 - r) + (1.0 - r);
            self.h = 1.0 / self.g.mul_add(self.k + self.g, 1.0);
        }
    }

    #[inline]
    pub fn tick(&mut self, x: f32) {
        let (g, k) = (self.g, self.k);
        let s7 = self.s1 + self.s1;
        let a = self.s2.mul_add(2.0, g * s7);
        let gg = g.mul_add(k, g * g);
        let b = s7.mul_add(k, a);
        let c = x.mul_add(gg, b);
        let hp = x - self.h * c;
        let v1 = g.mul_add(hp, self.s1);
        let bp = self.s1 + v1;
        let v2 = g.mul_add(bp, self.s2);
        self.lp = self.s2 + v2;
        self.bp = bp;
        self.s1 = v1;
        self.s2 = v2;
    }
}

/// `UTIL::BiQuad`: type 1 peaking bell, type 2 low shelf; `g2 = 2·tan(π·fc/sr)` is the
/// prewarped angle, `q` widens (`α = (1−q)·sin`), `gain` in dB.
#[derive(Clone, Copy)]
pub struct BiQuad {
    pub shelf: bool,
    pub z1: f32,
    pub z2: f32,
    pub g2: f32,
    pub a1: f32,
    pub a2: f32,
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub sr: f32,
    pub inv_sr: f32,
    pub fc: f32,
    pub q: f32,
    pub gain: f32,
}

impl BiQuad {
    pub fn new(shelf: bool) -> BiQuad {
        BiQuad {
            shelf,
            z1: 0.0,
            z2: 0.0,
            g2: 0.0,
            a1: 0.0,
            a2: 0.0,
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            sr: 1.0,
            inv_sr: 1.0,
            fc: 0.0,
            q: 0.0,
            gain: 0.0,
        }
    }

    /// Set the cutoff (clamped to `sr/24576 … sr/2.125`) and recompute.
    #[inline]
    pub fn set_fc(&mut self, fc: f32) {
        self.fc = fc;
        let lo = self.sr / 24576.0;
        let hi = self.sr / 2.125;
        let c = if lo > fc { lo } else { fc };
        let c = if c > hi { hi } else { c };
        let g = tan_w(self.inv_sr * (c * PI));
        self.g2 = g + g;
        self.calc();
    }

    /// `calculateCoefficients()`.
    pub fn calc(&mut self) {
        let g = self.g2;
        let g2 = g * g;
        let sp = g2.mul_add(f(0xb2cd_4ca8), f(0x3638_b94f));
        let sp = sp.mul_add(g2, f(0xb950_0c03));
        let sp = sp.mul_add(g2, f(0x3c08_8885));
        let sp = sp.mul_add(g2, f(0xbe2a_aac1));
        let sin = g * sp.mul_add(g2, 1.0);
        let cp = g2.mul_add(f(0xb48b_dad9), f(0x37cf_b5a1));
        let cp = cp.mul_add(g2, f(0xbab6_09bd));
        let cp = cp.mul_add(g2, f(0x3d2a_aa99));
        let cp = cp.mul_add(g2, f(0xbeff_f5e8));
        let cos = g2.mul_add(cp, 1.0);
        let alpha = (1.0 - self.q) * sin;
        let big_a = pow2_bq(self.gain / 40.0 * f(0x4054_9a80));
        if !self.shelf {
            let s4 = alpha / big_a;
            let s5 = 1.0 / (s4 + 1.0);
            let s2 = alpha * big_a;
            let s7 = s5 * -2.0;
            self.b0 = (s2 + 1.0) * s5;
            self.b1 = cos * s7;
            self.b2 = (1.0 - s2) * s5;
            self.a1 = -(cos * s7);
            self.a2 = -(s5 * (1.0 - s4));
        } else {
            let ap = big_a + 1.0;
            let am = big_a + -1.0;
            let sq = pow2_bq(big_a.ln() * f(0x3fb8_aa3b) * 0.5);
            let s0 = (alpha + alpha) * sq;
            let s1 = (-am).mul_add(cos, ap);
            let s2 = 1.0 / (s1 + s0);
            let s3 = am.mul_add(cos, ap);
            self.b0 = s2 * (big_a * (s3 + s0));
            self.b1 = big_a * (ap.mul_add(cos, am) * -2.0) * s2;
            self.b2 = s2 * (big_a * (s3 - s0));
            let t = (-ap).mul_add(cos, am);
            self.a1 = -(s2 * (t + t));
            self.a2 = -(s2 * (s1 - s0));
        }
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let w = self.a1.mul_add(self.z1, x);
        let w = self.a2.mul_add(self.z2, w);
        let y = self.b0.mul_add(w, self.z1 * self.b1);
        let y = self.b2.mul_add(self.z2, y);
        self.z2 = self.z1;
        self.z1 = w;
        y
    }
}

/// The retrigger stage that follows an envelope: at a note-on the previous output is
/// held and decays exponentially (time `time`, factor `2^(−3/(sr·time))`), mixed with
/// the new input (`mix` 0 → max, 1 → sum); once the output is below 1e-4 a cubic fade of
/// 15 ms closes it.
#[derive(Clone, Copy)]
pub struct Slew {
    pub time: f32,
    pub lvl: f32,
    pub coef: f32,
    pub out: f32,
    pub held: f32,
    pub mix: f32,
    pub inv_sr: f32,
    pub active: bool,
    pub n: u32,
    pub inv_n: f32,
    pub cnt: u32,
}

impl Slew {
    pub fn new(mix: f32) -> Slew {
        Slew { time: 1.0, lvl: 0.0, coef: 0.0, out: 0.0, held: 0.0, mix, inv_sr: 1.0, active: false, n: 1, inv_n: 0.0, cnt: 0 }
    }

    pub fn reset(&mut self) {
        self.out = 0.0;
        self.held = 0.0;
        self.lvl = 0.0;
        self.active = false;
        self.cnt = 0;
    }

    fn calc(&mut self) {
        let num = self.inv_sr * -3.0;
        let lim = self.inv_sr * f(0x4a00_2c80);
        let t = if self.time < THR { THR } else { self.time };
        let t = if lim < t { lim } else { t };
        let c = pow2_slew(num / t);
        self.coef = if c > f(0x3f7f_ff58) { f(0x3f7f_ff58) } else { c };
    }

    /// Set the hold time (recomputed when it changes).
    #[inline]
    pub fn set_time(&mut self, t: f32) {
        if self.time != t {
            self.time = t;
            self.calc();
        }
    }

    pub fn note_on(&mut self) {
        self.lvl = 1.0;
        self.held = self.out;
        self.active = true;
        self.cnt = 0;
    }

    /// The output and its fade factor.
    #[inline]
    pub fn tick(&mut self, x: f32) -> (f32, f32) {
        let held = self.lvl * self.held;
        self.lvl *= self.coef;
        let m = if held < x { x } else { held };
        let sum = x + held;
        let mix = if self.mix < 0.0 { 0.0 } else { self.mix };
        let mix = if mix > 1.0 { 1.0 } else { mix };
        let o = (sum - m).mul_add(mix, m);
        let o = if o < 0.0 { 0.0 } else { o };
        self.out = o;
        let mut fade = 1.0;
        if o < THR {
            let c = if self.cnt < self.n {
                self.cnt + 1
            } else {
                self.active = false;
                self.n
            };
            self.cnt = c;
            let v = (-(c as f32)).mul_add(self.inv_n, 1.0);
            fade = v * (v * v);
        }
        (o, fade)
    }
}

/// The clap hit envelope: a linear 0.1 ms attack, then `lvl *= coef` with
/// `coef = 2^(−1/(sr·time))` (time-to-half), offset by the 1e-4 floor.
#[derive(Clone, Copy)]
pub struct HitEnv {
    pub time: f32,
    pub lvl: f32,
    pub coef: f32,
    pub inv_sr: f32,
    pub phase: f32,
    pub rate: f32,
    pub attack: bool,
}

impl HitEnv {
    pub fn new() -> HitEnv {
        HitEnv { time: 1.0, lvl: 0.0, coef: 0.0, inv_sr: 1.0, phase: 0.0, rate: 0.0, attack: true }
    }

    pub fn calc(&mut self) {
        let x = (self.inv_sr * f(0xbf31_7215)) / (self.time + f(0x0da2_4260));
        let c = crate::fx::exp_e(x.mul_add(f(0x42ad_1f97), f(0x4496_0000)));
        let c = if c < 0.0 { 0.0 } else { c };
        self.coef = if c > 1.0 { 1.0 } else { c };
    }

    #[inline]
    pub fn set_time(&mut self, t: f32) {
        if self.time != t {
            self.time = t;
            self.calc();
        }
    }

    pub fn trigger(&mut self) {
        self.phase = 0.0;
        self.attack = true;
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        if self.attack {
            let p = self.rate + self.phase;
            self.phase = p;
            if p >= 1.0 {
                self.attack = false;
                self.lvl = 1.0;
                1.0
            } else {
                p
            }
        } else {
            let l = self.coef * self.lvl;
            self.lvl = l;
            if l < THR {
                self.lvl = 0.0;
                0.0
            } else {
                l - THR
            }
        }
    }
}

/// `ln(1/8)`.
const LN_EIGHTH: f64 = f64::from_bits(0xc000_a2b1_704f_f434);
/// `1e30`, a stage without time limit.
const NEVER: f64 = f64::from_bits(0x4629_3e59_39a0_8cea);
const TINY: f64 = f64::from_bits(0x3bc7_9ca1_0000_0000);
const END: f64 = f64::from_bits(0x3f1a_36e2_eb1c_432d);
const RESCALE: f64 = f64::from_bits(0x3ff0_0068_de3a_efe5);

/// `FreeEnvelope<off, breakpoint, Attack::shape, Decay::shape|log, Release::lin>`
/// (+0x318 and +0x410 in ClapCore). Offsets in comments are relative to the envelope.
#[derive(Clone, Copy)]
pub struct FreeEnv {
    /// Decay kind: `true` log (`<…,2,0>`), `false` shape (`<…,1,0>`).
    pub log: bool,
    /// +0x10 stage: 0 idle, 1 attack, 2 decay, 4 release.
    pub stage: u32,
    pub out: f32,
    pub x: f64,
    pub t: f64,
    pub tmax: f64,
    pub sub: f64,
    pub mul: f64,
    pub dec_coef: f64,
    pub rel_coef: f64,
    pub rel_level: f32,
    pub att_time: f64,
    pub inv_att: f64,
    pub dec_time: f64,
    pub inv_dec: f64,
    pub rel_time: f64,
    pub rel_raw: f64,
    pub inv_rel: f64,
    pub att_shape: f64,
    pub dec_shape: f64,
    pub rel_shape: f64,
    pub sustain: f32,
    pub gain: f32,
    pub vel_sens: f32,
    pub vel_gain: f32,
    pub att_set: f32,
    pub dec_set: f32,
    pub rel_set: f32,
    pub sr: f32,
    pub inv_sr: f64,
}

impl FreeEnv {
    /// The state `ClapCore()` leaves (shapes, sustain 0, velocity sensitivity 0).
    pub fn new(log: bool, att_shape: f32, dec_shape: f32) -> FreeEnv {
        FreeEnv {
            log,
            stage: 0,
            out: 0.0,
            x: 1.0,
            t: 0.0,
            tmax: 0.0,
            sub: 0.0,
            mul: 0.0,
            dec_coef: 0.0,
            rel_coef: 0.0,
            rel_level: 0.0,
            att_time: 1.0,
            inv_att: 1.0,
            dec_time: 1.0,
            inv_dec: 1.0,
            rel_time: 2.0 + TINY,
            rel_raw: 1.0,
            inv_rel: 1.0 / (2.0 + TINY),
            att_shape: att_shape as f64,
            dec_shape: dec_shape as f64,
            rel_shape: 0.0,
            sustain: 0.0,
            gain: 1.0,
            vel_sens: 0.0,
            vel_gain: 1.0,
            att_set: 0.0,
            dec_set: 0.0,
            rel_set: 0.0,
            sr: -1.0,
            inv_sr: 1.0,
        }
    }

    fn decay_coef(&self) -> f64 {
        if self.log {
            let s = self.dec_shape;
            let k = if 1.0 - s < 1e-7 { 10_000_000.0 } else { 1.0 / (1.0 - s) };
            let m = if s <= 0.0 { s + 1.0 } else { k };
            (LN_EIGHTH / (m * self.dec_time * self.sr as f64)).exp()
        } else {
            (LN_EIGHTH / (self.sr as f64 * self.dec_time)).exp()
        }
    }

    /// `setSamplerate(sr)` followed by the core's inlined `setReleaseTimeS(0)`.
    pub fn set_samplerate(&mut self, sr: f32) {
        self.sr = sr;
        let srd = sr as f64;
        self.inv_sr = 1.0 / srd;
        self.stage = 0;
        self.out = 0.0;
        self.x = 1.0;
        self.t = 0.0;
        let inv = self.inv_sr as f32;
        let a = self.att_time as f32;
        let t = if inv > a { inv } else { a };
        let t = if t > 100.0 { 100.0 } else { t };
        self.att_time = t as f64;
        self.inv_att = 1.0 / self.att_time;
        let d = self.dec_time as f32;
        self.att_set = a;
        self.dec_set = d;
        let t = if inv > d { inv } else { d };
        let t = if t > 100.0 { 100.0 } else { t };
        self.dec_time = t as f64;
        self.inv_dec = 1.0 / self.dec_time;
        self.dec_coef = self.decay_coef();
        // (setSamplerate's release part, then setReleaseTimeS(0) — rel_shape is 0.)
        self.rel_set = 0.0;
        self.rel_raw = 0.0;
        self.rel_coef = 0.0;
        let q = (-self.rel_shape).mul_add(self.rel_shape, 1.0);
        self.rel_time = 0.0 / (q * q) + TINY;
        self.inv_rel = 1.0 / self.rel_time;
    }

    /// `setAttackTimeS(v)`.
    pub fn set_attack(&mut self, v: f32) {
        if self.sr <= 0.0 || self.att_set == v {
            return;
        }
        self.att_set = v;
        let inv = self.inv_sr as f32;
        let t = if v < inv { inv } else { v };
        let t = if t > 100.0 { 100.0 } else { t };
        self.att_time = t as f64;
        self.inv_att = 1.0 / self.att_time;
    }

    /// `setDecayTimeS(v)` as inlined in the audio loop.
    #[inline]
    pub fn set_decay(&mut self, v: f32) {
        if self.dec_set == v {
            return;
        }
        self.dec_set = v;
        let inv = self.inv_sr as f32;
        let t = if v < inv { inv } else { v };
        let t = if t > 100.0 { 100.0 } else { t };
        self.dec_time = t as f64;
        self.inv_dec = 1.0 / self.dec_time;
        self.dec_coef = self.decay_coef();
        if self.stage == 2 {
            if self.log {
                self.sub = 0.0;
                self.mul = self.dec_coef;
            } else {
                self.sub = self.inv_sr * self.inv_dec;
                self.mul = 1.0;
            }
            self.tmax = if self.sustain == 0.0 { NEVER } else { self.dec_time };
        }
    }

    /// Back to idle (`onReset`).
    pub fn reset(&mut self) {
        self.stage = 0;
        self.out = 0.0;
        self.x = 1.0;
        self.t = 0.0;
    }

    pub fn note_on(&mut self, vel: f32) {
        if !(vel >= 0.0 && vel < f(0x3f80_0347)) {
            return;
        }
        self.reset();
        self.advance();
        self.vel_gain = self.vel_sens.mul_add(vel * vel, 1.0 - self.vel_sens);
    }

    fn advance(&mut self) {
        self.x = 1.0;
        self.t = 0.0;
        match self.stage {
            0 => {
                self.mul = 1.0;
                self.stage = 1;
                self.tmax = self.att_time;
                self.sub = self.inv_sr * self.inv_att;
            }
            1 => {
                if self.log {
                    self.sub = 0.0;
                    self.mul = self.dec_coef;
                } else {
                    self.sub = self.inv_sr * self.inv_dec;
                    self.mul = 1.0;
                }
                self.stage = 2;
                self.tmax = if self.sustain == 0.0 { NEVER } else { self.dec_time };
            }
            2 | 3 => {
                self.rel_level = self.out;
                if self.rel_shape == -1.0 {
                    self.sub = 0.0;
                    self.mul = self.rel_coef;
                } else {
                    self.sub = self.inv_sr * self.inv_rel;
                    self.mul = 1.0;
                }
                self.tmax = NEVER;
                self.stage = 4;
            }
            4 => {
                self.sub = 0.0;
                self.mul = 1.0;
                self.stage = 0;
                self.out = 0.0;
                self.x = 1.0;
            }
            _ => {}
        }
    }

    #[inline]
    fn step(&mut self) -> bool {
        let x = (self.x - self.sub) * self.mul;
        let t = self.inv_sr + self.t;
        let x = if x < 0.0 { 0.0 } else { x };
        let x = if x > 1.0 { 1.0 } else { x };
        self.x = x;
        self.t = t;
        x <= END || t > self.tmax
    }

    /// The attack/decay curve for shape `s` (hyperbolic for |s|, then an S-bend for s>0).
    #[inline]
    fn shape(y: f64, s: f64) -> f64 {
        let mut d2 = y;
        if s != 0.0 {
            let lim = f64::from_bits(0x3fef_f7ce_d916_872b);
            let a = s.abs();
            let a = if a > lim { lim } else { a };
            let d6 = (a + 1.0) * (a + 1.0) + -1.0;
            let d7 = 1.0 / d6;
            let b = 1.0 - a;
            let d16 = b * b;
            let ib = 1.0 / b;
            let ib = ib * ib;
            let (p, q) = if a <= 0.0 { (d6, d7) } else { (ib, d16) };
            let d4 = q + 1.0;
            let v = d2 + d4;
            let v = (-v).mul_add(p, p);
            d2 = d4 + d4 / v;
        }
        if s > 0.0 {
            let t = d2 + -0.5;
            let t = (-t).mul_add(t.abs(), t);
            d2 = t.mul_add(2.0, 0.5);
        }
        d2
    }

    /// `processSample()`.
    #[inline]
    pub fn process(&mut self) -> f32 {
        match self.stage {
            0 => {}
            1 => {
                let done = self.step();
                self.out = Self::shape(1.0 - self.x, self.att_shape) as f32;
                if done {
                    self.x = 1.0;
                    self.advance();
                }
            }
            2 => {
                let done = self.step();
                let d = self.x + -END;
                let s = self.sustain as f64;
                self.out = if self.log {
                    let r = if d < 0.0 { 0.0 } else { d * RESCALE };
                    (1.0 - s).mul_add(r, s) as f32
                } else {
                    let y = if d < 0.0 { 1.0 } else { d * -RESCALE + 1.0 };
                    let c = 1.0 - Self::shape(y, self.dec_shape);
                    (1.0 - s).mul_add(c, s) as f32
                };
                if done {
                    self.x = 1.0;
                    self.advance();
                }
            }
            _ => {
                let done = self.step();
                let d = self.x + -END;
                let r = if d < 0.0 { 0.0 } else { d * RESCALE };
                let r = if self.rel_shape == -1.0 { r } else { (r + -1.0) + 1.0 };
                self.out = (r * self.rel_level as f64) as f32;
                if done {
                    self.x = 1.0;
                    self.advance();
                }
            }
        }
        self.out * (self.vel_gain * self.gain)
    }
}
