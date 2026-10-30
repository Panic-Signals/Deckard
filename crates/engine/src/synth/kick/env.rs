//! `NI::DSP::UTIL::FreeEnvelope` (docs/synth-kick.md §3.1): the drum synths' attack /
//! decay / release envelope. Its state runs in `f64`: a phase `x` falls from 1 towards 0
//! in each stage, linearly (`x -= step`) or exponentially (`x *= coef`), and a stage ends
//! when `x ≤ 1e-4` or its time runs out. The template arguments (smoother, sustain,
//! attack, decay and release kinds) are [`Kinds`] here. Every operation follows the
//! assembly (`FreeEnvelope_impl.h`, e.g. `<1,1,1,2,0>` at 0x10197b978…0x10197bd24).

/// `-2.07944` (`0xc000a2b1704ff434`): `ln(1/8)`, so a log stage falls to 1/8 in its time.
pub const LN_EIGHTH: f64 = f64::from_bits(0xc000_a2b1_704f_f434);
/// The "never" stage length, `1e30` (`0x46293e5939a08cea`).
pub const NEVER: f64 = f64::from_bits(0x4629_3e59_39a0_8cea);
/// `1e-20` as `f32` widened (`0x3bc79ca100000000`).
const TINY: f64 = f64::from_bits(0x3bc7_9ca1_0000_0000);
/// The stage-end threshold, `1e-4` (`0x3f1a36e2eb1c432d`).
pub const END: f64 = f64::from_bits(0x3f1a_36e2_eb1c_432d);
/// `1/(1 − 1e-4)` (`0x3ff00068de3aefe5`).
const RESCALE: f64 = f64::from_bits(0x3ff0_0068_de3a_efe5);

/// Attack kinds (`FreeEnvelopeAttack`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Attack {
    /// 1: linear, bent by `att_shape` (`lin`).
    Lin,
    /// 2: the same with the bend inverted (`shape_inverted`).
    Inverted,
}

/// Decay kinds (`FreeEnvelopeDecay`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Decay {
    /// 0: linear.
    Lin,
    /// 1: linear with a curve shape (`dec_shape`).
    Shape,
    /// 2: exponential.
    Log,
}

/// Release kinds (`FreeEnvelopeRelease`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Release {
    /// 0: linear, or exponential when `rel_shape == -1`.
    Lin,
    /// 2: exponential. (Kind 1, a shaped release, is not used by the Kick.)
    Log,
}

/// A FreeEnvelope. Field comments give the member offset in the C++ object.
#[derive(Clone, Debug)]
pub struct FreeEnv {
    pub attack_kind: Attack,
    pub decay_kind: Decay,
    pub release_kind: Release,
    /// `FreeEnvelopeSustain` 2: the decay may stop at the sustain level (stage 3) while
    /// `hold` (+0xac) is set, until the next `advance`.
    pub gated: bool,
    pub hold: bool,
    /// +0x10: 0 idle, 1 attack, 2 decay, 3 sustain (gated), 4 release.
    pub stage: u32,
    /// +0x14: the last level (before velocity and gain).
    pub out: f32,
    /// +0x18: the stage phase, 1 → 0.
    pub x: f64,
    /// +0x20: time in the stage (s).
    pub t: f64,
    /// +0x28: the stage's length (s).
    pub tmax: f64,
    /// +0x30, +0x38: `x = (x − sub)·mul` per sample.
    pub sub: f64,
    pub mul: f64,
    /// +0x40, +0x48: exponential decay / release factors.
    pub dec_coef: f64,
    pub rel_coef: f64,
    /// +0x50: the level the release starts from.
    pub rel_level: f32,
    /// +0x58…+0x88: stage times (s) and their inverses; `rel_raw` the release time set.
    pub att_time: f64,
    pub inv_att: f64,
    pub dec_time: f64,
    pub inv_dec: f64,
    pub rel_time: f64,
    pub rel_raw: f64,
    pub inv_rel: f64,
    /// +0x90, +0x98, +0xa0: curve shapes.
    pub att_shape: f64,
    pub dec_shape: f64,
    pub rel_shape: f64,
    /// +0xa8: sustain level.
    pub sustain: f32,
    /// +0xb4: output gain; +0xb8 velocity sensitivity; +0xbc velocity gain.
    pub gain: f32,
    pub vel_sens: f32,
    pub vel_gain: f32,
    /// +0xe0, +0xe4, +0xe8: the times last set (the setters skip equal values).
    pub att_set: f32,
    pub dec_set: f32,
    pub rel_set: f32,
    /// +0xec, +0xf0: sample rate and its inverse.
    pub sr: f32,
    pub inv_sr: f64,
    /// `Smoother::on`: a one-pole low-pass on the level. +0xc0 state, +0xc4 output,
    /// +0xcc…+0xd4 coefficients, +0xd8 cutoff (Hz; −1 off).
    pub smooth: bool,
    pub sm_z: f32,
    pub sm_out: f32,
    pub sm_a: f32,
    pub sm_b: f32,
    pub sm_g: f32,
    pub sm_hz: f32,
}

impl FreeEnv {
    /// The constructor's state (all fields as `FreeEnvelope()` leaves them).
    pub fn new(decay_kind: Decay, release_kind: Release) -> FreeEnv {
        FreeEnv {
            attack_kind: Attack::Lin,
            decay_kind,
            release_kind,
            gated: false,
            hold: false,
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
            att_shape: 0.0,
            dec_shape: 0.0,
            rel_shape: 0.0,
            sustain: 0.5,
            gain: 1.0,
            vel_sens: 1.0,
            vel_gain: 1.0,
            att_set: 0.0,
            dec_set: 0.0,
            rel_set: 0.0,
            sr: -1.0,
            inv_sr: 1.0,
            smooth: false,
            sm_z: 0.0,
            sm_out: 0.0,
            sm_a: 0.0,
            sm_b: 0.0,
            sm_g: 0.0,
            sm_hz: 9999.0,
        }
    }

    /// Smoother cutoff (`setSmoothFreqHz`, before `setSamplerate`).
    pub fn set_smooth_hz(&mut self, hz: f32) {
        self.smooth = true;
        self.sm_hz = hz;
    }

    /// The exponential decay factor for the current decay time (`setDecayTimeS`).
    fn log_decay_coef(&self) -> f64 {
        let s = self.dec_shape;
        let k = if 1.0 - s < 1e-7 { 10_000_000.0 } else { 1.0 / (1.0 - s) };
        let m = if s <= 0.0 { s + 1.0 } else { k };
        (LN_EIGHTH / (m * self.dec_time * self.sr as f64)).exp()
    }

    /// `setSamplerate(sr)`: re-derives the stage times from the stored ones.
    pub fn set_samplerate(&mut self, sr: f32) {
        self.sr = sr;
        let srd = sr as f64;
        self.inv_sr = 1.0 / srd;
        let f = self.sm_hz;
        self.sm_hz = -1.0;
        if f != -1.0 {
            let w = (1.0 / sr) * (f * std::f32::consts::PI);
            let g = super::sub::tan_approx(w);
            self.sm_g = g;
            self.sm_hz = f;
            self.sm_a = 1.0 / (g + 1.0);
            self.sm_b = g + -1.0;
        }
        self.sm_z = 0.0;
        self.sm_out = 0.0;
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
        match self.decay_kind {
            Decay::Log => {
                let s = self.dec_shape;
                let k = if 1.0 - s < 1e-7 { 10_000_000.0 } else { 1.0 / (1.0 - s) };
                let m = if s <= 0.0 { s + 1.0 } else { k };
                self.dec_coef = (LN_EIGHTH / (m * self.dec_time * srd)).exp();
            }
            Decay::Shape | Decay::Lin => {
                self.dec_coef = (LN_EIGHTH / (srd * self.dec_time)).exp();
            }
        }
        let r = self.rel_time as f32;
        self.rel_set = r;
        self.rel_raw = r as f64;
        match self.release_kind {
            Release::Log => self.rel_coef = (LN_EIGHTH / (srd * self.rel_raw)).exp(),
            Release::Lin => {
                if self.rel_shape == -1.0 {
                    self.rel_coef = (LN_EIGHTH / (srd * self.rel_raw)).exp();
                } else {
                    self.rel_coef = 0.0;
                    let q = (-self.rel_shape).mul_add(self.rel_shape, 1.0);
                    self.rel_time = (self.rel_raw + self.rel_raw) / (q * q) + TINY;
                    self.inv_rel = 1.0 / self.rel_time;
                }
            }
        }
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

    /// `setDecayTimeS(v)`.
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
        match self.decay_kind {
            Decay::Log => {
                self.dec_coef = self.log_decay_coef();
                if self.stage == 2 {
                    self.sub = 0.0;
                    self.mul = self.dec_coef;
                    self.tmax = if (self.gated && self.hold) || self.sustain == 0.0 { NEVER } else { self.dec_time };
                }
            }
            Decay::Shape | Decay::Lin => {
                self.dec_coef = (LN_EIGHTH / (self.sr as f64 * self.dec_time)).exp();
                if self.stage == 2 {
                    self.sub = self.inv_sr * self.inv_dec;
                    self.mul = 1.0;
                    self.tmax = if (self.gated && self.hold) || self.sustain == 0.0 { NEVER } else { self.dec_time };
                }
            }
        }
    }

    /// `setReleaseTimeS(v)`.
    pub fn set_release(&mut self, v: f32) {
        if self.rel_set == v {
            return;
        }
        self.rel_set = v;
        self.rel_raw = v as f64;
        let exp = match self.release_kind {
            Release::Log => true,
            Release::Lin => self.rel_shape == -1.0,
        };
        if exp {
            self.rel_coef = (LN_EIGHTH / (self.rel_raw * self.sr as f64)).exp();
            if self.stage == 4 {
                self.mul = self.rel_coef;
            }
        } else {
            self.rel_coef = 0.0;
            let q = (-self.rel_shape).mul_add(self.rel_shape, 1.0);
            self.rel_time = (self.rel_raw + self.rel_raw) / (q * q) + TINY;
            self.inv_rel = 1.0 / self.rel_time;
            if self.stage == 4 {
                self.sub = self.inv_rel * self.inv_sr;
            }
        }
    }

    /// `setNoteOn(velocity)`: restart from the attack.
    pub fn note_on(&mut self, vel: f32) {
        if !(0.0..1.0001).contains(&vel) {
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
    pub fn advance(&mut self) {
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
                match self.decay_kind {
                    Decay::Log => {
                        self.sub = 0.0;
                        self.mul = self.dec_coef;
                    }
                    Decay::Shape | Decay::Lin => {
                        self.sub = self.inv_sr * self.inv_dec;
                        self.mul = 1.0;
                    }
                }
                self.stage = 2;
                self.tmax = if (self.gated && self.hold) || self.sustain == 0.0 { NEVER } else { self.dec_time };
            }
            2 if self.gated && self.hold => {
                self.mul = 1.0;
                self.stage = 3;
                self.tmax = NEVER;
                self.sub = 0.0;
                self.out = self.sustain;
            }
            2 | 3 => {
                self.rel_level = self.out;
                let exp = match self.release_kind {
                    Release::Log => true,
                    Release::Lin => self.rel_shape == -1.0,
                };
                if exp {
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

    /// Step `x` and `t`; whether the stage is over.
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

    /// `x` rescaled so 1e-4 → 0.
    #[inline]
    fn rescaled(&self) -> f64 {
        let d = self.x + -END;
        if d < 0.0 { 0.0 } else { d * RESCALE }
    }

    /// The curve bend of a shaped stage (`shape` ≠ 0): `a` is the clamped shape.
    #[inline]
    fn bend(y: f64, a: f64) -> f64 {
        let k = (a + 1.0) * (a + 1.0) + -1.0;
        let m = 1.0 / k;
        let (k, m) = if a <= 0.0 {
            (k, m)
        } else {
            let q = 1.0 - a;
            ((1.0 / q) * (1.0 / q), q * q)
        };
        let m = m + 1.0;
        m + m / (-(y + m)).mul_add(k, k)
    }

    /// The S-curve applied for positive shapes.
    #[inline]
    fn s_curve(y: f64) -> f64 {
        let y = y + -0.5;
        (-y).mul_add(y.abs(), y).mul_add(2.0, 0.5)
    }

    /// `processSample()`: the next level × velocity gain × gain.
    #[inline]
    pub fn process(&mut self) -> f32 {
        match self.stage {
            0 => {}
            1 => {
                let done = self.step();
                let mut y = 1.0 - self.x;
                let s = self.att_shape;
                if s != 0.0 {
                    let a = match self.attack_kind {
                        Attack::Lin => {
                            let a = s.abs();
                            if a > 0.999 { 0.999 } else { a }
                        }
                        Attack::Inverted => {
                            let a = s.abs();
                            let a = if a > 0.999 { -0.999 } else { -a };
                            if a > 0.999 { 0.999 } else { a }
                        }
                    };
                    y = Self::bend(y, a);
                }
                if s > 0.0 {
                    y = Self::s_curve(y);
                }
                self.out = y as f32;
                if done {
                    self.x = 1.0;
                    self.advance();
                }
            }
            2 => {
                let done = self.step();
                let s = self.sustain as f64;
                self.out = match self.decay_kind {
                    Decay::Log => (1.0 - s).mul_add(self.rescaled(), s) as f32,
                    Decay::Lin => {
                        let d = self.x + -END;
                        let y = if d < 0.0 { 0.0 } else { (d * RESCALE + -1.0) + 1.0 };
                        (1.0 - s).mul_add(y, s) as f32
                    }
                    Decay::Shape => {
                        let d = self.x + -END;
                        let mut y = if d < 0.0 { 1.0 } else { d * -RESCALE + 1.0 };
                        let sh = self.dec_shape;
                        if sh != 0.0 {
                            let a = sh.abs();
                            y = Self::bend(y, if a > 0.999 { 0.999 } else { a });
                        }
                        if sh > 0.0 {
                            y = Self::s_curve(y);
                        }
                        (1.0 - s).mul_add(1.0 - y, s) as f32
                    }
                };
                if done {
                    self.x = 1.0;
                    self.advance();
                }
            }
            3 => {
                if self.step() {
                    self.x = 1.0;
                    self.advance();
                }
            }
            _ => {
                let done = self.step();
                let y = self.rescaled();
                let y = match self.release_kind {
                    Release::Log => y,
                    Release::Lin => {
                        if self.rel_shape == -1.0 {
                            y
                        } else {
                            (y + -1.0) + 1.0
                        }
                    }
                };
                self.out = (y * self.rel_level as f64) as f32;
                if done {
                    self.x = 1.0;
                    self.advance();
                }
            }
        }
        if self.smooth {
            let y = self.sm_a * (-self.sm_b).mul_add(self.sm_z, self.out);
            let o = self.sm_g * (self.sm_z + y);
            self.sm_z = y;
            self.sm_out = o;
            return o * (self.vel_gain * self.gain);
        }
        self.out * (self.vel_gain * self.gain)
    }
}
