//! `NI::DSP::UTIL::FreeEnvelope<Smoother::off, Sustain::breakpoint, Attack::lin,
//! Decay::log, Release::lin>` (docs/synth-hihat.md §3.2), the drum synths' amplitude
//! envelope, as the hi-hat cores configure it: linear attack, exponential ("log") decay
//! to a breakpoint sustain, linear release. Its state is a falling ramp `x` (1 → 0) in
//! `f64`; the stages map `x` to the level. Field comments give the member offsets.

/// The stages (`+0x10`).
const IDLE: u32 = 0;
const ATTACK: u32 = 1;
const DECAY: u32 = 2;
const RELEASE: u32 = 4;

/// `1e30` (`0x46293e5939a08cea`): "no time limit".
pub(super) const FOREVER: f64 = f64::from_bits(0x4629_3e59_39a0_8cea);
/// `−ln 8` (`0xc000a2b1704ff434`): the decay coefficient's exponent per time constant.
pub(super) const LN8: f64 = f64::from_bits(0xc000_a2b1_704f_f434);
/// `1e-4` (`0x3f1a36e2eb1c432d`).
const END: f64 = f64::from_bits(0x3f1a_36e2_eb1c_432d);

#[derive(Clone, Debug)]
pub(super) struct FreeEnv {
    stage: u32,     // +0x10
    pub val: f32,   // +0x14
    x: f64,         // +0x18
    t: f64,         // +0x20
    tmax: f64,      // +0x28
    off: f64,       // +0x30
    mul: f64,       // +0x38
    dcoef: f64,     // +0x40
    relstart: f32,  // +0x50
    att: f64,       // +0x58
    att_inv: f64,   // +0x60
    att_f: f32,     // +0xe0: the attack time last set (f32)
    pub dec: f64,   // +0x68
    rel_inv: f64,   // +0x88
    dcurve: f64,    // +0x98
    rcurve: f64,    // +0xa0
    sus: f32,       // +0xa8
    gain: f32,      // +0xb4
    vsens: f32,     // +0xb8
    vgain: f32,     // +0xbc
    pub dec_f: f32, // +0xe4: the decay time last set (f32)
    sr: f32,        // +0xec
    inv: f64,       // +0xf0: 1/sr
}

impl FreeEnv {
    /// The configuration the hi-hat cores end up with after construction and
    /// `onSetSampleRate`: attack `att` s, decay set later per sample, sustain 0, an
    /// instantaneous linear release (its time is forced to 0), level `gain`, no
    /// velocity sensitivity.
    pub fn new(sr: f32, att: f32, gain: f32) -> FreeEnv {
        let inv = 1.0 / sr as f64;
        // setSamplerate: attack/decay clamped to [1/sr, 100] (f32), from their previous
        // values (attack 1.0 → then the core's 0.0013 s; decay 1.0).
        let att = (att.max(inv as f32)).min(100.0) as f64;
        let dec = (1f32.max(inv as f32)).min(100.0) as f64;
        // Release time 0 → 0/(1−0)² + 1e-20 (`0x3bc79ca100000000`).
        let rel = 0.0 / 1.0 + f64::from_bits(0x3bc7_9ca1_0000_0000);
        let mut e = FreeEnv {
            stage: IDLE,
            val: 0.0,
            x: 1.0,
            t: 0.0,
            tmax: 0.0,
            off: 0.0,
            mul: 1.0,
            dcoef: 0.0,
            relstart: 0.0,
            att,
            att_inv: 1.0 / att,
            att_f: att as f32,
            dec,
            rel_inv: 1.0 / rel,
            dcurve: 0.0,
            rcurve: 0.0,
            sus: 0.0,
            gain,
            vsens: 0.0,
            vgain: 1.0,
            dec_f: dec as f32,
            sr,
            inv,
        };
        e.dcoef = (LN8 / (e.k() * dec * sr as f64)).exp();
        e
    }

    /// The decay-time factor from the decay curve (`+0x98`): `curve + 1` for `curve ≤ 0`,
    /// else `1/(1 − curve)` (1e7 when that is below 1e-7).
    fn k(&self) -> f64 {
        let c = self.dcurve;
        let d2 = 1.0 - c;
        let d3 = 1.0 / d2;
        let d2 = if d2 < 1e-7 { 1e7 } else { d3 };
        if c <= 0.0 { c + 1.0 } else { d2 }
    }

    /// Whether it runs (stage ≠ idle).
    pub fn active(&self) -> bool {
        self.stage != IDLE
    }

    /// A new decay time in seconds (`f32`), as the cores set it every sample while their
    /// coefficients are fresh: clamped to [1/sr, 100], `dcoef = exp(−ln 8/(k·T·sr))`, and
    /// live when decaying.
    pub fn set_decay(&mut self, t: f32) {
        if self.dec_f == t {
            return;
        }
        self.dec_f = t;
        let s1 = self.inv as f32;
        let s0 = if t < s1 { s1 } else { t };
        let s0 = if s0 > 100.0 { 100.0 } else { s0 };
        let d9 = s0 as f64;
        self.dec = d9;
        let c = (LN8 / (self.k() * d9 * self.sr as f64)).exp();
        self.dcoef = c;
        if self.stage == DECAY {
            self.off = 0.0;
            self.mul = c;
            self.tmax = if self.sus == 0.0 { FOREVER } else { d9 };
        }
    }

    /// `setNoteOn(v)`: restart from the attack.
    pub fn note_on(&mut self, v: f32) {
        self.stage = IDLE;
        self.val = 0.0;
        self.x = 1.0;
        self.t = 0.0;
        self.advance();
        self.vgain = (1.0 - self.vsens) + v * v * self.vsens;
    }

    /// A new attack time (seconds): it applies from the next note (the running stage
    /// keeps its step).
    pub fn set_attack(&mut self, t: f32) {
        if self.att_f == t {
            return;
        }
        self.att_f = t;
        let s1 = self.inv as f32;
        let s = if t < s1 { s1 } else { t };
        let s = if s > 100.0 { 100.0 } else { s };
        self.att = s as f64;
        self.att_inv = 1.0 / self.att;
    }

    /// The level (`+0xb4`).
    pub fn set_gain(&mut self, g: f32) {
        self.gain = g;
    }

    /// The cores' reset: idle, level 0.
    pub fn reset(&mut self) {
        self.stage = IDLE;
        self.val = 0.0;
        self.x = 1.0;
        self.t = 0.0;
    }

    /// `advanceStage()`.
    fn advance(&mut self) {
        self.x = 1.0;
        self.t = 0.0;
        match self.stage {
            IDLE => {
                self.mul = 1.0;
                self.stage = ATTACK;
                self.tmax = self.att;
                self.off = self.inv * self.att_inv;
            }
            ATTACK => {
                self.off = 0.0;
                self.mul = self.dcoef;
                self.stage = DECAY;
                self.tmax = if self.sus == 0.0 { FOREVER } else { self.dec };
            }
            2 | 3 => {
                self.relstart = self.val;
                if self.rcurve == -1.0 {
                    unreachable!("exponential release is not configured");
                }
                self.off = self.inv * self.rel_inv;
                self.mul = 1.0;
                self.tmax = FOREVER;
                self.stage = RELEASE;
            }
            RELEASE => {
                self.mul = 1.0;
                self.off = 0.0;
                self.stage = IDLE;
                self.val = 0.0;
            }
            _ => {}
        }
    }

    /// One sample (`processSampleStage<stage>`).
    pub fn tick(&mut self) -> f32 {
        if self.stage == IDLE {
            return self.val * (self.vgain * self.gain);
        }
        let d1 = (self.x - self.off) * self.mul;
        let t = self.inv + self.t;
        let d1 = if d1 < 0.0 { 0.0 } else { d1 };
        let x = if d1 > 1.0 { 1.0 } else { d1 };
        self.x = x;
        self.t = t;
        match self.stage {
            ATTACK => {
                // Attack curve 0: linear.
                self.val = (1.0 - x) as f32;
            }
            DECAY => {
                let d = x + -END;
                let d = if d < 0.0 { 0.0 } else { d * f64::from_bits(0x3ff0_0068_de3a_efe5) };
                let s = self.sus as f64;
                self.val = (1.0 - s).mul_add(d, s) as f32;
            }
            _ => {
                let d = x + -END;
                let d = if d < 0.0 { 0.0 } else { d * f64::from_bits(0x3ff0_0068_de3a_efe5) };
                let d = if self.rcurve == -1.0 { d } else { (d + -1.0) + 1.0 };
                self.val = (d * self.relstart as f64) as f32;
            }
        }
        if x <= END || t > self.tmax {
            self.x = 1.0;
            self.advance();
        }
        self.val * (self.vgain * self.gain)
    }
}
