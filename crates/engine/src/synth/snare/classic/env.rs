//! `NI::DSP::UTIL::FreeEnvelope` as `ClassicSnareCore` instantiates it (docs/synth-snare.md):
//!
//! - the amp envelope `<Smoother::off, Sustain::breakpoint, Attack::shape_inverted,
//!   Decay::lin, Release::shape>` (+0x802d0 in the core), and
//! - the noise envelope `<off, breakpoint, Attack::lin, Decay::lin, Release::log>` (+0x803c8).
//!
//! The state runs in `f64`: a phase `x` falls from 1 towards 0 in each stage, linearly
//! (`x = (x − sub)·mul` with `mul = 1`) or exponentially (`sub = 0`), and a stage ends when
//! `x ≤ 1e-4` or its time runs out. Every operation follows the assembly
//! (`processSampleStage<0|1|2|4>` at 0x1019689a0…/0x101961b78…, `advanceStage`,
//! `setSamplerate` 0x1018d07dc / 0x1018b1654, `setNoteOn` 0x1018d1a70 / 0x1018b1850).

/// `ln(1/8)` (`0xc000a2b1704ff434`): a log stage falls to 1/8 in its time.
pub(super) const LN8: f64 = f64::from_bits(0xc000_a2b1_704f_f434);
/// The "never" stage length, `1e30`.
pub(super) const NEVER: f64 = f64::from_bits(0x4629_3e59_39a0_8cea);
/// The stage-end threshold `1e-4` (also the core's other envelopes' threshold).
pub(super) const END: f64 = f64::from_bits(0x3f1a_36e2_eb1c_432d);
/// `1/(1 − 1e-4)`.
const RESCALE: f64 = f64::from_bits(0x3ff0_0068_de3a_efe5);
/// `1e-20` as `f32` widened.
pub(super) const TINY: f64 = f64::from_bits(0x3bc7_9ca1_0000_0000);
/// The largest curve amount, `0.99999`.
const MAX_SHAPE: f64 = f64::from_bits(0x3fef_f7ce_d916_872b);

/// One FreeEnvelope. Field comments give the member offset in the C++ object.
#[derive(Clone)]
pub(super) struct FreeEnv {
    /// Template: `Attack::shape_inverted` + `Release::shape` (amp) or `Attack::lin` +
    /// `Release::log` (noise).
    pub amp_kind: bool,
    /// +0x10 stage (0 idle, 1 attack, 2 decay, 4 release); +0x14 last level.
    pub stage: u32,
    pub out: f32,
    /// +0x18 phase, +0x20 time in stage, +0x28 stage limit, +0x30/+0x38 step / factor.
    pub x: f64,
    pub t: f64,
    pub tmax: f64,
    pub sub: f64,
    pub mul: f64,
    /// +0x40 decay factor (unused by `Decay::lin`), +0x48 release factor.
    pub dec_coef: f64,
    pub rel_coef: f64,
    /// +0x50 level the release starts from.
    pub rel_level: f32,
    /// +0x58…+0x88 stage times and inverses; `rel_raw` the release time as set.
    pub att_time: f64,
    pub inv_att: f64,
    pub dec_time: f64,
    pub inv_dec: f64,
    pub rel_time: f64,
    pub rel_raw: f64,
    pub inv_rel: f64,
    /// +0x90 attack curve, +0xa0 release curve (−1: exponential release).
    pub att_shape: f64,
    pub rel_shape: f64,
    /// +0xa8 sustain level (a breakpoint: the decay stage lasts `dec_time` unless 0).
    pub sustain: f32,
    /// +0xb4 gain, +0xb8 velocity sensitivity, +0xbc velocity gain.
    pub gain: f32,
    pub vel_sens: f32,
    pub vel_gain: f32,
    /// +0xe0/+0xe4/+0xe8 times as last set (f32).
    pub att_set: f32,
    pub dec_set: f32,
    pub rel_set: f32,
    /// +0xec sample rate, +0xf0 its inverse.
    pub sr: f32,
    pub inv_sr: f64,
}

/// Attack / release curve (`shape` ≠ 0) and the S-curve on top (`shape` > 0).
#[inline]
fn curve(mut y: f64, c: f64, inverted: bool) -> f64 {
    if c != 0.0 {
        let a = c.abs();
        let s = if inverted {
            if a > MAX_SHAPE { -MAX_SHAPE } else { -a }
        } else if a > MAX_SHAPE {
            MAX_SHAPE
        } else {
            a
        };
        let p = (s + 1.0) * (s + 1.0) + -1.0;
        let rp = 1.0 / p;
        let n = 1.0 - s;
        let n2 = n * n;
        let rn = 1.0 / n;
        let rn2 = rn * rn;
        let (k, j) = if s <= 0.0 { (p, rp) } else { (rn2, n2) };
        let j1 = j + 1.0;
        let z = y + j1;
        let z = (-z).mul_add(k, k);
        y = j1 + j1 / z;
    }
    if c > 0.0 {
        let h = y + -0.5;
        let h = (-h).mul_add(h.abs(), h);
        y = h.mul_add(2.0, 0.5);
    }
    y
}

impl FreeEnv {
    /// The constructor state (`ClassicSnareCore()` then its overrides).
    pub fn new(amp_kind: bool) -> FreeEnv {
        FreeEnv {
            amp_kind,
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
            rel_time: 1.0,
            rel_raw: 1.0,
            inv_rel: 1.0,
            // amp: 0.2f widened; noise: linear.
            att_shape: if amp_kind { 0.2f32 as f64 } else { 0.0 },
            rel_shape: if amp_kind { 0.0 } else { -1.0 },
            sustain: if amp_kind { 0.5 } else { 1.0 },
            gain: if amp_kind { 1.0 } else { f32::from_bits(0x3f19_999a) },
            vel_sens: 0.0,
            vel_gain: 1.0,
            att_set: 0.0,
            dec_set: 0.0,
            rel_set: 0.0,
            sr: -1.0,
            inv_sr: 1.0,
        }
    }

    /// `setSamplerate(sr)`.
    pub fn set_samplerate(&mut self, sr: f32) {
        if sr <= 0.0 || sr.is_nan() {
            return;
        }
        self.sr = sr;
        let srd = sr as f64;
        self.inv_sr = 1.0 / srd;
        self.stage = 0;
        self.out = 0.0;
        self.x = 1.0;
        self.t = 0.0;
        let lo = self.inv_sr as f32;
        let a = self.att_time as f32;
        let t = if lo > a { lo } else { a };
        let t = if t > 100.0 { 100.0 } else { t };
        self.att_time = t as f64;
        self.inv_att = 1.0 / self.att_time;
        let d = self.dec_time as f32;
        self.att_set = a;
        self.dec_set = d;
        let t = if lo > d { lo } else { d };
        let t = if t > 100.0 { 100.0 } else { t };
        self.dec_time = t as f64;
        self.inv_dec = 1.0 / self.dec_time;
        self.dec_coef = (LN8 / (srd * self.dec_time)).exp();
        let r = self.rel_time as f32;
        self.rel_set = r;
        self.rel_raw = r as f64;
        if !self.amp_kind || self.rel_shape == -1.0 {
            self.rel_coef = (LN8 / (srd * self.rel_raw)).exp();
        } else {
            self.rel_coef = 0.0;
            let q = (-self.rel_shape).mul_add(self.rel_shape, 1.0);
            self.rel_time = (self.rel_raw + self.rel_raw) / (q * q) + TINY;
            self.inv_rel = 1.0 / self.rel_time;
        }
    }

    /// The attack-time setter `onSetSampleRate` inlines (`time` in s, if changed).
    pub fn set_attack(&mut self, time: f32) {
        if self.sr <= 0.0 || self.att_set == time {
            return;
        }
        self.att_set = time;
        let t = (self.inv_sr as f32).max(time);
        let t = if t > 100.0 { 100.0 } else { t };
        self.att_time = t as f64;
        self.inv_att = 1.0 / self.att_time;
    }

    /// The decay-time setter the core inlines per sample (`time` in s, if changed).
    pub fn set_decay(&mut self, time: f32) {
        if self.dec_set == time {
            return;
        }
        self.dec_set = time;
        let lo = self.inv_sr as f32;
        let t = if time < lo { lo } else { time };
        let t = if t > 100.0 { 100.0 } else { t };
        let d = t as f64;
        let r = 1.0 / d;
        self.dec_time = d;
        self.inv_dec = r;
        self.dec_coef = (LN8 / (self.sr as f64 * d)).exp();
        if self.stage == 2 {
            self.mul = 1.0;
            self.tmax = if self.sustain == 0.0 { NEVER } else { d };
            self.sub = self.inv_sr * r;
        }
    }

    /// The release-time setter the core inlines per sample.
    pub fn set_release(&mut self, time: f32) {
        if self.rel_set == time {
            return;
        }
        self.rel_set = time;
        let d = time as f64;
        self.rel_raw = d;
        if !self.amp_kind || self.rel_shape == -1.0 {
            let e = (LN8 / (d * self.sr as f64)).exp();
            self.rel_coef = e;
            if self.stage == 4 {
                self.mul = e;
            }
        } else {
            self.rel_coef = 0.0;
            let q = (-self.rel_shape).mul_add(self.rel_shape, 1.0);
            self.rel_time = (d + d) / (q * q) + TINY;
            self.inv_rel = 1.0 / self.rel_time;
            if self.stage == 4 {
                self.sub = self.inv_rel * self.inv_sr;
            }
        }
    }

    /// `setNoteOn(velocity)` (0 ≤ velocity < 1.0001): restart from the attack.
    pub fn note_on(&mut self, vel: f32) {
        if !(0.0..f32::from_bits(0x3f80_0347)).contains(&vel) {
            return;
        }
        self.stage = 0;
        self.out = 0.0;
        self.x = 1.0;
        self.t = 0.0;
        self.advance();
        self.vel_gain = self.vel_sens.mul_add(vel * vel, 1.0 - self.vel_sens);
    }

    /// `advanceStage()`.
    fn advance(&mut self) {
        self.x = 1.0;
        self.t = 0.0;
        match self.stage {
            0 => {
                self.sub = self.inv_sr * self.inv_att;
                self.mul = 1.0;
                self.stage = 1;
                self.tmax = self.att_time;
            }
            1 => {
                self.sub = self.inv_sr * self.inv_dec;
                self.mul = 1.0;
                self.stage = 2;
                self.tmax = if self.sustain == 0.0 { NEVER } else { self.dec_time };
            }
            2 | 3 => {
                self.rel_level = self.out;
                if !self.amp_kind || self.rel_shape == -1.0 {
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

    /// One sample: the level × (velocity gain × gain).
    #[inline]
    pub fn process(&mut self) -> f32 {
        if self.stage == 0 {
            return self.out * (self.vel_gain * self.gain);
        }
        let x = (self.x - self.sub) * self.mul;
        let t = self.inv_sr + self.t;
        let x = if x < 0.0 { 0.0 } else { x };
        let x = if x > 1.0 { 1.0 } else { x };
        self.x = x;
        self.t = t;
        self.out = match self.stage {
            1 => curve(1.0 - x, self.att_shape, self.amp_kind) as f32,
            2 => {
                let e = x + -END;
                let r = if e < 0.0 { 0.0 } else { e * RESCALE + -1.0 + 1.0 };
                let s = self.sustain as f64;
                (1.0 - s).mul_add(r, s) as f32
            }
            _ => {
                let e = x + -END;
                let r = if e < 0.0 { 0.0 } else { e * RESCALE };
                let l = if !self.amp_kind || self.rel_shape == -1.0 { r } else { 1.0 - curve(1.0 - r, self.rel_shape, false) };
                (l * self.rel_level as f64) as f32
            }
        };
        if x <= END || t > self.tmax {
            self.x = 1.0;
            self.advance();
        }
        self.out * (self.vel_gain * self.gain)
    }
}
