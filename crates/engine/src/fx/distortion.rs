//! Maschine's Distortion (docs/fx-distortion.md), class `DATA::MullhollandDrive`. Its Mode
//! picks one of two cores held side by side in `RT::MullhollandDriveProcessor`; only the
//! chosen one runs, and the other keeps its state:
//!
//! - **Mulholland** (mode 0): `FXCORE::MullhollandDrive`, hand-vectorised C++ in four NEON
//!   lanes. A state-variable filter with a fractional feedback delay ("Feedback", "Tone",
//!   "Tone Mod"), a huge input gain, two one-pole high-passes and a pre-emphasis shelf, a
//!   ×4 Lagrange-interpolated soft clipper `k·x/(k + |x|)`, the matching de-emphasis,
//!   two low-pass biquad pairs crossfaded by Color, an envelope gate and a dry/wet mix.
//!   Parameters glide over 50 ms in control ticks of ≈ 0.1 ms.
//! - **Analog** (mode 1): `NI::DSP::AnalogDistortion`, run at 176.4/192 kHz between two
//!   integer-ratio `NI::AB::SampleRateConverter`s: an envelope-following bias ("Focus")
//!   into a 256-point waveshaper table, one-pole tone filters, a DC blocker, a dry/wet mix.
//!
//! All arithmetic follows the assembly operation by operation (`f32`; `mul_add` exactly where
//! Maschine uses a fused multiply-add), so the output can match bit for bit.

use super::{Effect, exp_e, param};

/// An `f32` constant as Maschine stores it.
const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// Field names of `DATA::MullhollandDrive` and their defaults (stored values). Mode is
/// missing from class-version-0 archives, which Maschine loads as Mulholland (§1.2).
const PARAMS: [(&str, f32); 15] = [
    ("mode", 0.0),
    ("drive", 0.25),
    ("color", 0.25),
    ("feedback", 0.25),
    ("tone", 0.75),
    ("tone_mod", 0.0),
    ("gate", 1.0),
    ("release", 0.5),
    ("mix", 0.5),
    ("analog_saturation", 0.25),
    ("analog_bass", 1.0),
    ("analog_tone", 1.0),
    ("analog_focus", 0.0),
    ("analog_definition", 0.5),
    ("analog_mix", 1.0),
];

/// NEON `FRECPE` (8-bit reciprocal estimate) of each lane.
#[cfg(target_arch = "aarch64")]
fn frecpe4(x: [f32; 4]) -> [f32; 4] {
    use std::arch::aarch64::*;
    // SAFETY: NEON is always present on aarch64.
    unsafe {
        let mut o = [0f32; 4];
        vst1q_f32(o.as_mut_ptr(), vrecpeq_f32(vld1q_f32(x.as_ptr())));
        o
    }
}

/// ARM's `FRECPE` for normal, finite `x` (what NEON computes on aarch64).
#[cfg(not(target_arch = "aarch64"))]
fn frecpe4(x: [f32; 4]) -> [f32; 4] {
    x.map(|x| {
        let b = x.to_bits();
        let (sign, exp, frac) = (b & 0x8000_0000, ((b >> 23) & 0xff) as i32, b & 0x7f_ffff);
        if exp == 0 || exp == 0xff {
            return 1.0 / x;
        }
        let a = (0x100 | (frac >> 15)) * 2 + 1;
        let est = ((1u32 << 19) / a).div_ceil(2);
        let mut f = (est & 0xff) << 15;
        let mut re = 253 - exp;
        if re == 0 {
            f = 0x40_0000 | (f >> 1);
        } else if re == -1 {
            f = 0x20_0000 | (f >> 2);
            re = 0;
        } else if re < -1 {
            return f32::from_bits(sign);
        }
        f32::from_bits(sign | ((re as u32) << 23) | f)
    })
}

/// `1/x` as the kernel computes it: `FRECPE`, then two `FRECPS` steps (`2 − x·r`, fused).
fn recip4(x: [f32; 4]) -> [f32; 4] {
    let r = frecpe4(x);
    let mut o = [0f32; 4];
    for k in 0..4 {
        let r1 = r[k] * (-x[k]).mul_add(r[k], 2.0);
        o[k] = (-x[k]).mul_add(r1, 2.0) * r1;
    }
    o
}

/// `round()` of the setup code: add ±0.5 (by the sign of the `f32`) in `f64`, truncate.
fn round_away(x: f32) -> i32 {
    let h = if x < 0.0 { -0.5 } else { 0.5 };
    (x as f64 + h) as i32
}

// ---------------------------------------------------------------------------------------
// Mulholland: FXCORE::MullhollandDrive (§3)
// ---------------------------------------------------------------------------------------

/// `2^x` (§3.1): `frinti` (round to nearest even), a quadratic for the fraction, and the
/// integer part put into the exponent bits.
fn pow2(x: f32) -> f32 {
    let n = x.round_ties_even();
    let i = if n < 2_147_483_648.0 && n > -2_147_483_648.0 { n as i32 } else { i32::MIN };
    let f = x - i as f32;
    let p = f.mul_add(c(0x3e7d_f3b6), c(0x3f35_3f7d)); // 0.248, 0.708
    p.mul_add(f, 1.0) * f32::from_bits(0x3f80_0000u32.wrapping_add((i as u32) << 23))
}

/// The output gain's exponent (log2): `(−16 − 7.2·drive + 9·u(2 − u) − 6·|fb|)·log2(10)/20`,
/// `u = 1.5·c` (§3.2).
fn gain_log2(drive: f32, col: f32, fbs: f32) -> f32 {
    let u = col * 1.5;
    let u = u * (2.0 - u);
    let t = (drive * c(0x3f19_999a)).mul_add(-12.0, -16.0);
    let t = u.mul_add(9.0, t);
    let t = fbs.abs().mul_add(-6.0, t);
    t * c(0x3e2a_152d)
}

/// The Release time in seconds (`scaleReleaseValue`, §1.4).
fn release_secs(v: f32) -> f32 {
    let s = v * v;
    let t = s * s - s;
    let s = t.mul_add(c(0x3f4f_41f2), s); // 0.8096
    s.mul_add(c(0x409f_ae14), c(0x3c23_d70a)) // 4.99, 0.01
}

/// `exp(−ln 10 / max(n − 1, ·))`: the coefficient that falls 20 dB in `n` samples.
fn decay(n: f32) -> f32 {
    let m = n + -1.0;
    let x = if m < 1.0 { c(0xc013_5d8d) } else { c(0xc013_5d8d) / m };
    x.exp()
}

/// The 11-point tables of the feedback filter's frequency and damping, indexed by
/// `370 Hz / Nyquist · 10` (`setSampleRate`'s `s_freqTable`, `s_dampTable`).
const FREQ_TABLE: [u32; 11] = [
    0x0000_0000,
    0x3e90_46c7,
    0x3f01_00e7,
    0x3f2c_840e,
    0x3f4c_8fda,
    0x3f62_fe1e,
    0x3f71_9c39,
    0x3f7a_2bbf,
    0x3f7e_5e89,
    0x3f7f_d0d0,
    0x3f80_0000,
];
const DAMP_TABLE: [u32; 11] = [
    0x4000_0000,
    0x3fdb_ee4e,
    0x3fbf_7f8d,
    0x3fa9_bdf9,
    0x3f99_b813,
    0x3f8e_80f1,
    0x3f87_31e4,
    0x3f82_ea21,
    0x3f80_d0bb,
    0x3f80_1798,
    0x3f80_0000,
];

/// Lagrange weights of the ×4 interpolator and decimator (§3.3).
const W0703: f32 = c(0x3d90_0000); // 0.0703125
const W8672: f32 = c(0x3f5e_0008); // 0.867188
const W2266: f32 = c(0x3e68_0022); // 0.226563
const W0234: f32 = c(0x3cc0_0000); // 0.0234375
const W0625: f32 = c(0x3d80_0000); // 0.0625
const W5625: f32 = 0.5625;

type V4 = [f32; 4];

/// Ramp indices (§3.2): the 20 parameters that glide.
const R_DRIVE: usize = 0; // input gain (Drive)
const R_GAIN: usize = 1; // output gain
const R_LPF: usize = 2; // Color's low-pass frequency
const R_XF: usize = 3; // Color's crossfade
const R_MIX: usize = 4;
const R_FB: usize = 5; // feedback amount
const R_DLY: usize = 6; // feedback delay (samples)
const R_SH_C: usize = 8; // shelves: 1/(t+1) ×2, t − 1 ×2, t ×2, e² − 1 ×2, k ×4

/// `FXCORE::MullhollandDrive` (§3). Field comments give the object offsets.
struct Mulholland {
    // Parameters as set (+0x00..+0x1c) and their derived values.
    drive: f32,
    color: f32,
    fb: f32,
    tone: f32,
    mix: f32,
    tone_mod: f32,
    release: f32,
    gate: f32,
    /// Color, shaped (+0x20).
    col: f32,
    /// Feedback, shaped `v·(0.5 + 0.5|v|)·|…|` (+0x24).
    fbs: f32,
    /// Tone: the feedback delay in seconds (+0x28).
    tone_s: f32,
    /// Tone Mod: delay = `tm_a·env + tm_b` seconds on top of Tone (+0x2c, +0x30).
    tm_a: f32,
    tm_b: f32,
    /// The delayed output fed back (+0x40, lanes 0/1).
    fbv: [f32; 2],
    /// The feedback SVF's frequency and damping (+0x50, +0x54) and states (+0x60, +0x70).
    svf_f: f32,
    svf_d: f32,
    s1: V4,
    s2: V4,
    /// The two one-pole high-passes: `1/(t+1)` and `t − 1` at 370 and 25 Hz (+0x80..).
    hp_c: [f32; 2],
    hp_t: [f32; 2],
    hp1: V4,
    hp2: V4,
    /// Pre-emphasis and de-emphasis shelf states (+0xd0, +0xe0).
    pre: V4,
    de: V4,
    /// The ×4 interpolator's input history and the four clipped phases (+0x100..+0x23f).
    hist: [V4; 4],
    ph: [[V4; 4]; 4],
    ri: usize,
    /// Biquad history (+0x250..+0x2cf), toggled by `tog`.
    bx1: [V4; 2],
    by1: [V4; 2],
    bx2: [V4; 2],
    by2: [V4; 2],
    tog: usize,
    /// Biquad coefficients b1, b0, a1, a2 (+0x2f0, +0x300, +0x2d0, +0x2e0).
    b1: V4,
    b0: V4,
    a1: V4,
    a2: V4,
    /// The feedback delay line (stereo frames, +0x340), write index (floats), int/frac delay.
    buf: Vec<f32>,
    w: usize,
    di: i32,
    frac: f32,
    /// Gate envelope: attack/release coefficients, value, hold length and counter.
    att: f32,
    rel: f32,
    genv: f32,
    hold: i32,
    hcnt: i32,
    /// The 20 glides (+0x380 value, +0x400 start, +0x480 delta, +0x500 progress).
    val: [f32; 20],
    start: [f32; 20],
    delta: [f32; 20],
    prog: [f32; 20],
    step: f32,
    sr: f32,
    isr: f32,
    w2pi: f32,
    maxd: f32,
    /// Control tick countdown and period (+0x594, +0x598).
    ctl: i32,
    ctl_n: i32,
    /// The level follower (+0x59c) and its coefficient (+0x5a0).
    env: f32,
    env_c: f32,
}

impl Mulholland {
    /// The constructor's state, then `setSampleRate(sr)` (the processor's constructor uses
    /// 44100).
    fn new(sr: f32) -> Box<Mulholland> {
        let mut m = Box::new(Mulholland {
            drive: 0.0,
            color: 0.5,
            fb: 0.0,
            tone: 0.5,
            mix: 0.5,
            tone_mod: 0.0,
            release: 0.5,
            gate: 1.0,
            col: 0.0,
            fbs: 0.0,
            tone_s: 0.0,
            tm_a: 0.0,
            tm_b: 0.0,
            fbv: [0.0; 2],
            svf_f: 0.0,
            svf_d: 0.0,
            s1: [0.0; 4],
            s2: [0.0; 4],
            hp_c: [0.0; 2],
            hp_t: [0.0; 2],
            hp1: [0.0; 4],
            hp2: [0.0; 4],
            pre: [0.0; 4],
            de: [0.0; 4],
            hist: [[0.0; 4]; 4],
            ph: [[[0.0; 4]; 4]; 4],
            ri: 0,
            bx1: [[0.0; 4]; 2],
            by1: [[0.0; 4]; 2],
            bx2: [[0.0; 4]; 2],
            by2: [[0.0; 4]; 2],
            tog: 0,
            b1: [0.0; 4],
            b0: [0.0; 4],
            a1: [0.0; 4],
            a2: [0.0; 4],
            buf: Vec::new(),
            w: 0,
            di: 0,
            frac: 0.0,
            att: 0.0,
            rel: 0.0,
            genv: 0.0,
            hold: 1,
            hcnt: 0,
            val: [0.0; 20],
            start: [0.0; 20],
            delta: [0.0; 20],
            prog: [1.0; 20],
            step: 0.0,
            sr: 0.0,
            isr: 0.0,
            w2pi: 0.0,
            maxd: 0.0,
            ctl: 1,
            ctl_n: 1,
            env: 0.0,
            env_c: 0.0,
        });
        m.set_sample_rate(sr);
        m
    }

    /// Restart glide `i` toward `t` from where it is now (every setter does this).
    fn retarget(&mut self, i: usize, t: f32) {
        let s = self.delta[i].mul_add(self.prog[i], self.start[i]);
        self.start[i] = s;
        self.delta[i] = t - s;
        self.prog[i] = 0.0;
        self.val[i] = s;
    }

    fn gain_target(&self) -> f32 {
        pow2(gain_log2(self.drive, self.col, self.fbs))
    }

    /// `setDrive`: input gain `40 + 99960·(0.6·d·(0.34 + 0.66d))³` and the output gain.
    fn set_drive(&mut self, v: f32) {
        self.drive = v;
        let g = v.mul_add(c(0x3f28_f5c3), c(0x3eae_147b)) * v * c(0x3f19_999a);
        let g = g * (g * g);
        let g = g.mul_add(c(0x47c3_3c00), 40.0);
        let o = self.gain_target();
        self.retarget(R_DRIVE, g);
        self.retarget(R_GAIN, o);
    }

    /// `setColor` (§3.2): the emphasis shelves, the clipper's knee, the Color low-pass and
    /// the crossfade between the two biquad paths.
    fn set_color(&mut self, v: f32) {
        self.color = v;
        let x = v + v + -1.0;
        let a = x.abs().mul_add(c(0x3e99_999a), c(0x3e4c_cccd)); // 0.3, 0.2
        let col = a.mul_add(x, 0.5);
        self.col = col;
        let xf = col * (2.0 - col);
        let m = col + col;
        let m = if m > 1.0 { 1.0 } else { m };
        let db = m.mul_add(-28.0, -12.0);
        let lpf = m.mul_add(c(0xc5a3_e000), c(0x460c_a000)); // −5244, 9000
        let e0 = pow2(db * c(0xbdaa_152d)); // 10^(−db/40)
        let e1 = pow2(db * c(0x3daa_152d)); // 10^(db/40)
        for (k, t) in [e0, e0, e1, e1].into_iter().enumerate() {
            self.retarget(R_SH_C + 8 + k, t);
        }
        self.retarget(R_XF, xf);
        let o = self.gain_target();
        self.retarget(R_GAIN, o);
        self.retarget(R_LPF, lpf);
        let lim = self.sr * c(0x3efc_ed91); // 0.494·sr
        let f0 = 1200.0 / e0;
        let f0 = if lim < f0 { lim } else { f0 };
        let f1 = 1200.0 / e1;
        let f1 = if lim < f1 { lim } else { f1 };
        let pi = c(0x4049_0fdb);
        let t0 = (self.isr * (f0 * pi)).tan();
        let t1 = (self.isr * (f1 * pi)).tan();
        let tg = [1.0 / (t0 + 1.0), 1.0 / (t1 + 1.0), t0 + -1.0, t1 + -1.0, t0, t1, e0 * e0 + -1.0, e1 * e1 + -1.0];
        for (k, t) in tg.into_iter().enumerate() {
            self.retarget(R_SH_C + k, t);
        }
    }

    fn shape_fb(v: f32) -> f32 {
        let s = v.abs().mul_add(0.5, 0.5) * v;
        s * s.abs()
    }

    /// `setFB`: the feedback amount (`0.3·fbs`) and the output gain.
    fn set_fb(&mut self, v: f32) {
        self.fb = v;
        self.fbs = Self::shape_fb(v);
        self.retarget(R_FB, self.fbs * c(0x3e99_999a));
        let o = self.gain_target();
        self.retarget(R_GAIN, o);
    }

    /// `setToneFB`: the feedback delay `0.02899·v² + 1e-5` seconds.
    fn set_tone(&mut self, v: f32) {
        self.tone = v;
        self.tone_s = (v * v).mul_add(c(0x3ced_7c70), c(0x3727_c5ac));
    }

    fn set_mix(&mut self, v: f32) {
        self.mix = v;
        self.retarget(R_MIX, v);
    }

    /// `setMove` (Tone Mod): `a = −0.1·v|v|·|v|v|`, `b = max(−a, 0)`.
    fn set_tone_mod(&mut self, v: f32) {
        self.tone_mod = v;
        let (a, b) = Self::tone_mod_ab(v);
        self.tm_a = a;
        self.tm_b = b;
    }

    fn tone_mod_ab(v: f32) -> (f32, f32) {
        let s = v.abs() * v;
        let t = s * c(0xbdcc_cccd);
        let a = t * s.abs();
        (a, if a > 0.0 { 0.0 } else { -(t * s.abs()) })
    }

    /// `setUnscaledRelease`: the gate's release falls 20 dB in `release_secs(v)`.
    fn set_release(&mut self, v: f32) {
        self.release = v;
        self.rel = decay(release_secs(v) * self.sr);
    }

    fn set_gate(&mut self, on: bool) {
        self.gate = if on { 1.0 } else { 0.0 };
    }

    /// `setSampleRate` (§3.4): rates, the delay line, the fixed filters, every target, and
    /// a reset of all signal state; the glides land at their targets on the next tick.
    fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr;
        self.isr = 1.0 / sr;
        self.w2pi = self.isr * c(0x40c9_0fdb);
        self.maxd = sr.mul_add(c(0x3ee8_3315), 1.0);
        let frames = ((round_away(self.maxd + 1.0) as i64 + 0x21) & !0x1f) as usize;
        self.buf = vec![0.0; 2 * frames];
        let x = 370.0 / (sr * 0.5) * 10.0;
        let xi = if x < 0.0 { x as f64 - 1.0 } else { x as f64 } as i32;
        let i = xi.clamp(0, 9) as usize;
        let f = x - i as f32;
        let one = 1.0 - f;
        self.svf_f = c(FREQ_TABLE[i]).mul_add(one, f * c(FREQ_TABLE[i + 1]));
        self.svf_d = c(DAMP_TABLE[i]).mul_add(one, f * c(DAMP_TABLE[i + 1]));
        let lim = sr * c(0x3efc_ed91);
        let lpi = lim * c(0x4049_0fdb);
        let t1 = ((if lim < 370.0 { lpi } else { c(0x4491_4c75) }) / sr).tan();
        let t2 = ((if lim < 25.0 { lpi } else { c(0x429d_1463) }) / sr).tan();
        self.hp_t = [t1 + -1.0, t2 + -1.0];
        self.hp_c = [1.0 / (t1 + 1.0), 1.0 / (t2 + 1.0)];
        let k = sr * c(0x3e36_427c);
        self.env_c = if k > 1.0 { c(0x3f31_7215) / k + -1.0 } else { c(0xbe9d_1bd6) };
        self.ctl_n = round_away(sr / 9800.0).max(1);
        self.step = 1.0 / (sr / self.ctl_n as f32 * c(0x3d4c_cccd));
        // Drive's two glides, then Color (which also sets the output gain).
        let d = self.drive;
        let g = d.mul_add(c(0x3f28_f5c3), c(0x3eae_147b)) * d * c(0x3f19_999a);
        let g = (g * (g * g)).mul_add(c(0x47c3_3c00), 40.0);
        let o = self.gain_target();
        self.retarget(R_DRIVE, g);
        self.retarget(R_GAIN, o);
        self.set_color(self.color);
        self.fbs = Self::shape_fb(self.fb);
        let o = self.gain_target();
        self.retarget(R_GAIN, o);
        self.tone_s = (self.tone * self.tone).mul_add(c(0x3ced_7c70), c(0x3727_c5ac));
        // Mix and feedback glides (their progress is not cleared here).
        for (i, t) in [(R_MIX, self.mix), (R_FB, self.fbs * c(0x3e99_999a))] {
            let s = self.delta[i].mul_add(self.prog[i], self.start[i]);
            self.start[i] = s;
            self.delta[i] = t - s;
            self.val[i] = s;
        }
        let (a, b) = Self::tone_mod_ab(self.tone_mod);
        self.tm_a = a;
        self.tm_b = b;
        let a40 = c(0x3d23_d70a) * sr;
        self.att = decay(a40);
        self.rel = decay(release_secs(self.release) * sr);
        self.hold = round_away(a40).max(1);
        self.w = 0;
        self.fbv = [0.0; 2];
        self.s1 = [0.0; 4];
        self.s2 = [0.0; 4];
        self.hp1 = [0.0; 4];
        self.hp2 = [0.0; 4];
        self.pre = [0.0; 4];
        self.de = [0.0; 4];
        self.ri = 0;
        self.tog = 0;
        self.hist = [[0.0; 4]; 4];
        self.ph = [[[0.0; 4]; 4]; 4];
        self.bx1 = [[0.0; 4]; 2];
        self.by1 = [[0.0; 4]; 2];
        self.bx2 = [[0.0; 4]; 2];
        self.by2 = [[0.0; 4]; 2];
        self.prog = [1.0; 20];
        self.ctl = 1;
        self.env = 0.0;
        self.genv = 0.0;
        self.hcnt = 0;
    }

    /// The control tick (§3.5): the feedback delay follows the level, the glides advance,
    /// and the four low-pass biquads are redesigned.
    fn tick(&mut self) {
        self.ctl = self.ctl_n;
        let d = self.tm_a.mul_add(self.env, self.tm_b) + self.tone_s;
        let d = d * self.sr;
        let d = if d < 0.0 { 0.0 } else { d };
        self.retarget(R_DLY, d);
        for i in 0..20 {
            let p = self.step + self.prog[i];
            let p = if p < 1.0 { p } else { 1.0 };
            self.prog[i] = p;
            self.val[i] = self.start[i] + self.delta[i] * p;
        }
        let dl = self.val[R_DLY];
        let dl = if dl < 0.0 { 0.0 } else { dl };
        let dl = if dl > self.maxd { self.maxd } else { dl };
        self.di = dl as i32;
        self.frac = dl - self.di as f32;
        // Low-passes at 8150 Hz, the Color frequency f, 10863.9 Hz and 1.333·f (Q 0.714).
        let f = self.val[R_LPF];
        let hz = [c(0x45fe_b000), f, c(0x4629_bf9a), c(0x3faa_9fbe) * f];
        let mut sn = [0f32; 4];
        let mut cs = [0f32; 4];
        for k in 0..4 {
            let w = hz[k] * self.w2pi;
            let w = if w < c(0x4039_999a) { w } else { c(0x4039_999a) }; // 2.9
            let x = w * w;
            let mut p = x * c(0xb2cd_4ca8) + c(0x3638_b94f);
            p = x * p + c(0xb950_0c03);
            p = x * p + c(0x3c08_8885);
            p = x * p + c(0xbe2a_aac1);
            p = x * p + 1.0;
            sn[k] = w * p;
            let mut q = x * c(0xb48b_dad9) + c(0x37cf_b5a1);
            q = x * q + c(0xbab6_09bd);
            q = x * q + c(0x3d2a_aa99);
            q = x * q + c(0xbeff_f5e8);
            cs[k] = x * q + 1.0;
        }
        let al = sn.map(|s| s * c(0x3f33_3333)); // sin·0.7
        let r = recip4([al[0] + 1.0, al[1] + 1.0, al[2] + 1.0, al[3] + 1.0]);
        for k in 0..4 {
            let omc = 1.0 - cs[k];
            self.b1[k] = r[k] * omc;
            self.b0[k] = r[k] * (omc * 0.5);
            self.a1[k] = r[k] * (cs[k] * -2.0);
            self.a2[k] = r[k] * (1.0 - al[k]);
        }
    }

    /// One stereo sample (`MullhollandDriveImpl::processAudio`, §3.3).
    fn sample(&mut self, l: f32, r: f32) -> (f32, f32) {
        let x = [l, r, l, r];
        let al = l.abs();
        let ar = r.abs();
        let peak = if al < ar { ar } else { al };
        let k = if peak > 0.0 { self.env_c } else { 0.0 };
        self.env = (peak - self.env).mul_add(k + 1.0, self.env);
        let n = self.ctl;
        self.ctl = n.wrapping_sub(1);
        if n < 2 {
            self.tick();
        }
        let v = self.val;
        // The feedback state-variable filter; lanes 2/3 have no feedback.
        let fba = v[R_FB];
        let fbv = [self.fbv[0], self.fbv[1], 0.0, 0.0];
        let mut s1 = [0f32; 4];
        let mut s2 = [0f32; 4];
        for k in 0..4 {
            let inp = x[k] + fbv[k] * fba;
            let hp = inp - (self.s1[k] * self.svf_d + self.s2[k]);
            s1[k] = self.s1[k] + hp * self.svf_f;
            s2[k] = self.s2[k] + s1[k] * self.svf_f;
        }
        self.s1 = s1;
        self.s2 = s2;
        // Band-pass (L, R) and low-pass (L, R) into two one-pole high-pass stages each.
        let y = [s1[0], s1[1], s2[0], s2[1]];
        let hc = [self.hp_c[0], self.hp_c[0], self.hp_c[1], self.hp_c[1]];
        let ht = [self.hp_t[0], self.hp_t[0], self.hp_t[1], self.hp_t[1]];
        let mut h = [0f32; 4];
        for k in 0..4 {
            let y1 = hc[k] * (y[k] * v[R_DRIVE] - self.hp1[k] * ht[k]);
            let y2 = hc[k] * ((y1 - self.hp1[k]) - self.hp2[k] * ht[k]);
            h[k] = y2 - self.hp2[k];
            self.hp1[k] = y1;
            self.hp2[k] = y2;
        }
        // Pre-emphasis shelves (gain e0² on lanes 0/1, e1² on 2/3 above the corner).
        let sh = &v[R_SH_C..R_SH_C + 12];
        let mut e = [0f32; 4];
        for k in 0..4 {
            let j = k / 2;
            let z = self.pre[k];
            let y = sh[j] * (h[k] - z * sh[2 + j]);
            self.pre[k] = y;
            e[k] = h[k] + sh[6 + j] * (sh[4 + j] * (z + y));
        }
        // ×4: Lagrange interpolation at −1.75, −1.5, −1.25 and −1 samples, each clipped
        // with k·x/(k + |x|).
        let i0 = self.ri;
        self.hist[i0] = e;
        let a = self.hist[(i0 + 1) & 3];
        let b = self.hist[(i0 + 2) & 3];
        let cc = self.hist[(i0 + 3) & 3];
        let kn = [sh[8], sh[9], sh[10], sh[11]];
        let mut p = [[0f32; 4]; 4];
        for k in 0..4 {
            p[0][k] = ((b[k] * W8672 - a[k] * W0703) + cc[k] * W2266) - e[k] * W0234;
            p[1][k] = ((b[k] * W5625 - a[k] * W0625) + cc[k] * W5625) - e[k] * W0625;
            p[2][k] = ((b[k] * W2266 - a[k] * W0234) + cc[k] * W8672) - e[k] * W0703;
            p[3][k] = cc[k];
        }
        for (j, pj) in p.iter().enumerate() {
            let den = [kn[0] + pj[0].abs(), kn[1] + pj[1].abs(), kn[2] + pj[2].abs(), kn[3] + pj[3].abs()];
            let rc = recip4(den);
            for k in 0..4 {
                self.ph[j][i0][k] = rc[k] * (kn[k] * pj[k]);
            }
        }
        // Back to the base rate: the same weights, mirrored, over the clipped phases.
        let (im1, im2, im3) = ((i0 + 3) & 3, (i0 + 2) & 3, (i0 + 1) & 3);
        let mut o = [0f32; 4];
        for (k, ok) in o.iter_mut().enumerate() {
            let q1 = ((self.ph[1][im1][k] * W8672 - self.ph[1][i0][k] * W0703) + self.ph[1][im2][k] * W2266) - self.ph[1][im3][k] * W0234;
            let q2 = ((self.ph[2][im1][k] * W5625 - self.ph[2][i0][k] * W0625) + self.ph[2][im2][k] * W5625) - self.ph[2][im3][k] * W0625;
            let q3 = ((self.ph[3][im1][k] * W2266 - self.ph[3][i0][k] * W0234) + self.ph[3][im2][k] * W8672) - self.ph[3][im3][k] * W0703;
            *ok = (((self.ph[0][im1][k] + q1) + q2) + q3) * 0.25;
        }
        self.ri = im3;
        // De-emphasis shelves (the mirror: e1² on lanes 0/1, e0² on 2/3).
        let v = self.val;
        let sh = &v[R_SH_C..R_SH_C + 8];
        let mut dm = [0f32; 4];
        for k in 0..4 {
            let j = 1 - k / 2;
            let z = self.de[k];
            let y = sh[j] * (o[k] - z * sh[2 + j]);
            self.de[k] = y;
            dm[k] = o[k] + sh[6 + j] * (sh[4 + j] * (z + y));
        }
        // Two biquad low-passes in series: 8150 → 10863.9 Hz (lanes 0/1), f → 1.333 f (2/3).
        let (jn, ko) = (self.tog, self.tog ^ 1);
        self.tog = ko;
        let lane = |c: &V4, h: usize, k: usize| c[2 * h + k / 2];
        let mut y1 = [0f32; 4];
        let mut y2 = [0f32; 4];
        for k in 0..4 {
            let t = self.bx1[jn][k] * lane(&self.b1, 0, k) + lane(&self.b0, 0, k) * (dm[k] + self.bx1[ko][k]);
            let t = t - lane(&self.a1, 0, k) * self.by1[jn][k];
            y1[k] = t - lane(&self.a2, 0, k) * self.by1[ko][k];
        }
        for k in 0..4 {
            let t = lane(&self.b1, 1, k) * self.bx2[jn][k] + lane(&self.b0, 1, k) * (y1[k] + self.bx2[ko][k]);
            let t = t - lane(&self.a1, 1, k) * self.by2[jn][k];
            y2[k] = t - lane(&self.a2, 1, k) * self.by2[ko][k];
        }
        self.bx1[ko] = dm;
        self.by1[ko] = y1;
        self.bx2[ko] = y1;
        self.by2[ko] = y2;
        // Color crossfades the 8150 Hz path into the Color path.
        let xf = v[R_XF];
        let wet = [y2[2] + (y2[0] - y2[2]) * xf, y2[3] + (y2[1] - y2[3]) * xf];
        // The feedback delay: write, then read `di + frac` frames back.
        let len = self.buf.len();
        self.buf[self.w] = wet[0];
        self.buf[self.w + 1] = wet[1];
        self.w += 2;
        if self.w >= len {
            self.w = 0;
        }
        let rd = self.w as i64 - 2 * self.di as i64;
        let at = |i: i64| if i < 0 { (i + len as i64) as usize } else { i as usize };
        let (n0, n1) = (at(rd - 2), at(rd - 4));
        for ch in 0..2 {
            let a = self.buf[n0 + ch];
            let b = self.buf[n1 + ch];
            self.fbv[ch] = a + (b - a) * self.frac;
        }
        // The gate: open (attack toward 1) while the level is above −40 dB or within the
        // 40 ms hold, else release toward 0.
        let mut cnt = self.hcnt;
        if peak > c(0x3c23_d70a) {
            cnt = self.hold;
            self.hcnt = cnt;
        }
        let g = if cnt > 0 {
            self.hcnt = cnt - 1;
            (-(1.0 - self.genv)).mul_add(self.att, 1.0)
        } else {
            self.genv.mul_add(self.rel, c(0x2193_92ef))
        };
        self.genv = g;
        let g = if self.gate != 0.0 { g } else { 1.0 };
        let m = v[R_MIX] * g;
        let d = 1.0 - m;
        let wg = m * (2.0 - m);
        let dg = d * (2.0 - d);
        let wg = v[R_GAIN] * wg;
        (l * dg + wet[0] * wg, r * dg + wet[1] * wg)
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            (*a, *b) = self.sample(*a, *b);
        }
    }
}

// ---------------------------------------------------------------------------------------
// Analog: NI::DSP::AnalogDistortion (§4) and its sample-rate converters (§5)
// ---------------------------------------------------------------------------------------

/// The converters' 16-tap windowed-sinc phase at fractional position `t` (`init_intrat`):
/// `h[i] = sinc(t + i − 8)·I0(8·√(1 − ((t + i − 8)/8)²)) / 427.564…` (Abramowitz–Stegun I0),
/// in `f64` like the original.
fn src_phase(t: f32) -> [f32; 16] {
    let mut h = [0f32; 16];
    let pi = f64::from_bits(0x4009_21fb_5444_2d11);
    for (i, o) in h.iter_mut().enumerate() {
        let x = t + (i as i32 - 8) as f32;
        if x == 0.0 {
            *o = 1.0;
            continue;
        }
        let s = (x as f64 * pi).sin();
        let u = ((x + x) * 0.0625) as f64;
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
        *o = ((s / (x as f64 * pi)) * i0 / f64::from_bits(0x407a_b906_bec5_d25f)) as f32;
    }
    h
}

/// One sample through a transposed 16-tap FIR: `out = Σ h[i]·x[n−i]`, summed from the
/// oldest tap, each product and sum rounded on its own.
fn fir(acc: &mut [f32; 16], h: &[f32; 16], x: f32) -> f32 {
    let out = acc[0] + x * h[0];
    for i in 0..15 {
        acc[i] = acc[i + 1] + x * h[i + 1];
    }
    acc[15] = 0.0;
    out
}

/// `SampleRateConverter` UP (integer ratio `R`, `process_intratUP`), one channel: per
/// input sample, the input from 8 samples ago, then `R − 1` FIR phases at `j/R`.
struct Up {
    h: Vec<[f32; 16]>,
    acc: Vec<[f32; 16]>,
    delay: [f32; 8],
    di: usize,
}

impl Up {
    fn new(r: usize) -> Up {
        let mut h = Vec::new();
        let mut t = 0f32;
        for _ in 1..r {
            t += 1.0 / r as f32;
            h.push(src_phase(t));
        }
        Up { acc: vec![[0.0; 16]; r - 1], h, delay: [0.0; 8], di: 0 }
    }

    fn run(&mut self, inp: &[f32], out: &mut [f32]) {
        let r = self.h.len() + 1;
        for (k, &x) in inp.iter().enumerate() {
            out[k * r] = self.delay[self.di];
            self.delay[self.di] = x;
            self.di = (self.di + 1) & 7;
            for j in 0..r - 1 {
                out[k * r + 1 + j] = fir(&mut self.acc[j], &self.h[j], x);
            }
        }
    }
}

/// `SampleRateConverter` DOWN (integer ratio `R`, `process_intratDOWN`), one channel: per
/// `R` inputs, one sample 8 groups old plus the `R − 1` FIR phases, times `1/R`.
struct Down {
    h: Vec<[f32; 16]>,
    acc: Vec<[f32; 16]>,
    delay: [f32; 8],
    di: usize,
    sum: f32,
    phase: usize,
    scale: f32,
}

impl Down {
    fn new(r: usize) -> Down {
        let mut h = Vec::new();
        let mut t = 1f32;
        for _ in 1..r {
            t += -1.0 / r as f32;
            h.push(src_phase(t));
        }
        Down { acc: vec![[0.0; 16]; r - 1], h, delay: [0.0; 8], di: 0, sum: 0.0, phase: r - 1, scale: 1.0 / r as f32 }
    }

    fn run(&mut self, inp: &[f32], out: &mut [f32]) {
        let r = self.h.len() + 1;
        let mut n = 0;
        for &x in inp {
            if self.phase == r - 1 {
                let d = self.delay[self.di];
                self.delay[self.di] = x;
                self.di = (self.di + 1) & 7;
                out[n] = (d + self.sum) * self.scale;
                n += 1;
                self.sum = 0.0;
                self.phase = 0;
            } else {
                let y = fir(&mut self.acc[self.phase], &self.h[self.phase], x);
                self.sum += y;
                self.phase += 1;
            }
        }
    }
}

/// `NI::DSP::AnalogDistortion`'s parameter glides (§4.3): linear over `ticks` control
/// ticks, stepping every processed sample; `dirty` restarts them at the next tick.
struct Glide<const N: usize> {
    cur: [f32; N],
    inc: [f32; N],
    tgt: [f32; N],
    dirty: bool,
    countdown: i32,
    ticks: i32,
    inv: f32,
}

impl<const N: usize> Glide<N> {
    fn new() -> Self {
        Glide { cur: [0.0; N], inc: [0.0; N], tgt: [0.0; N], dirty: false, countdown: 0, ticks: 0, inv: 0.0 }
    }

    fn step(&mut self) {
        for k in 0..N {
            self.cur[k] += self.inc[k];
        }
    }

    fn snap(&mut self) {
        self.cur = self.tgt;
        self.dirty = true;
    }

    fn tick(&mut self) {
        if self.dirty {
            self.dirty = false;
            let mut sum = 0f32;
            for k in 0..N {
                self.inc[k] = (self.tgt[k] - self.cur[k]) * self.inv;
                let sq = self.inc[k] * self.inc[k];
                if (sq as f64) < 1e-15 {
                    self.inc[k] = 0.0;
                    self.cur[k] = self.tgt[k];
                } else {
                    sum += sq;
                }
            }
            if sum > 0.0 {
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
}

/// Glide lanes of the filter parameters (+0x280.., targets +0x2e0..).
const G_BASS: usize = 0;
const G_FATT: usize = 1;
const G_FREL: usize = 2;
const G_FOCUS: usize = 3;
const G_SAT: usize = 4;
const G_SMOOTH: usize = 5;
const G_TONE: usize = 6;
const G_DEF: usize = 7;
const G_COMP: usize = 8;
const G_TRIM: usize = 9;

/// The control block: `MullhollandDriveProcessor` runs the core in chunks of at most 128
/// samples, with a control tick before each full one.
const BLOCK: usize = 128;

/// `NI::DSP::AnalogDistortion` (§4).
struct Analog {
    sr: f32,
    /// The oversampled rate (+0x2c): 176.4 or 192 kHz.
    fs: f32,
    /// Parameters 0..7 as set (+0x08..+0x24).
    p: [f32; 8],
    /// Filter glides (12 lanes) and the dry/wet glide (4 lanes).
    g: Glide<12>,
    mixg: Glide<4>,
    suspended: bool,
    /// Per channel: bass follower, waveshaper smoother, tone low-pass, dry tone low-pass,
    /// DC blocker (+0x390..+0x3b4); the shared focus envelope (+0x3bc), the DC coefficient.
    bass: [f32; 2],
    smooth: [f32; 2],
    lp: [f32; 2],
    dry_lp: [f32; 2],
    dc: [f32; 2],
    dc_c: f32,
    fenv: f32,
    /// The converters (when `fs ≠ sr`) and work buffers.
    up: Option<[Up; 2]>,
    down: Option<[Down; 2]>,
    wl: Vec<f32>,
    wr: Vec<f32>,
}

/// `E(x)·2π/fs`, capped at 1: a one-pole coefficient from an ExpTable pitch.
fn coef(x: f32, w: f32) -> f32 {
    (exp_e(x) * w).min(1.0)
}

impl Analog {
    /// The processor's construction: `onInit`, `onSetSampleRate(44100, 128)`, and the two
    /// fixed parameters (2 = 1.0, 6 = 0.9).
    fn new() -> Box<Analog> {
        let mut a = Box::new(Analog {
            sr: 0.0,
            fs: 0.0,
            p: [0.0; 8],
            g: Glide::new(),
            mixg: Glide::new(),
            suspended: true,
            bass: [0.0; 2],
            smooth: [0.0; 2],
            lp: [0.0; 2],
            dry_lp: [0.0; 2],
            dc: [0.0; 2],
            dc_c: 0.0,
            fenv: 0.0,
            up: None,
            down: None,
            wl: Vec::new(),
            wr: Vec::new(),
        });
        a.set_sample_rate(44100.0);
        a.set_param(2, 1.0);
        a.set_param(6, 0.9);
        a
    }

    /// Focus: attack/release `ln 2/(fs·t)` (t = 1..30 ms and 5..34 ms) and the amount
    /// `0.6u/(u + 0.2)`, `u = 1 − v`.
    fn set_focus(&mut self) {
        let u = 1.0 - self.p[4];
        let amt = u * c(0x3f99_999a) * 0.5 / (u + c(0x3f99_999a) + -1.0);
        let t = [u.mul_add(c(0x3ced_9168), c(0x3a83_126f)), u.mul_add(c(0x3ced_9168), c(0x3ba3_d70a))];
        for (k, t) in t.into_iter().enumerate() {
            let x = t * self.fs;
            self.g.tgt[G_FATT + k] = if x > 1.0 { c(0x3f31_7215) / x } else { c(0x3f31_7215) };
        }
        self.g.tgt[G_FOCUS] = amt;
    }

    /// `setFilters`: the Bass, smoothing and Tone low-passes and the DC blocker (4.6 Hz).
    fn set_filters(&mut self) {
        let w = c(0x40c9_0fdb) / self.fs;
        self.g.tgt[G_BASS] = coef((1.0 - self.p[1]).mul_add(110.0, 4.0).mul_add(5.0, c(0x44ac_bc36)), w);
        let s = self.p[2].mul_add(40.0, 130.0);
        let s = (-self.p[0]).mul_add(40.0, s);
        self.g.tgt[G_SMOOTH] = coef(s.mul_add(5.0, c(0x44ac_bc36)), w);
        self.g.tgt[G_TONE] = coef((self.p[3] * self.p[3]).mul_add(45.0, 67.0).mul_add(5.0, c(0x44ac_bc36)), w);
        self.g.dirty = true;
        let e = exp_e(1331.0 + c(0x3f61_b000)) as f64;
        self.dc_c = (f64::from_bits(0x4019_21fb_5444_2d18) / self.fs as f64 * e) as f32;
    }

    /// `setParameter` (§4.2).
    fn set_param(&mut self, idx: usize, v: f32) {
        let w = c(0x40c9_0fdb) / self.fs;
        let db = |x: f32| exp_e(x.mul_add(c(0x411f_73da), 1200.0));
        self.p[idx] = v;
        match idx {
            0 | 2 => {
                if idx == 0 {
                    self.g.tgt[G_SAT] = db(v.mul_add(40.0, 20.0));
                }
                let s = self.p[2].mul_add(40.0, 130.0);
                let s = (-self.p[0]).mul_add(40.0, s);
                self.g.tgt[G_SMOOTH] = coef(s.mul_add(5.0, c(0x44ac_bc36)), w);
            }
            1 => self.g.tgt[G_BASS] = coef((1.0 - v).mul_add(110.0, 4.0).mul_add(5.0, c(0x44ac_bc36)), w),
            3 => self.g.tgt[G_TONE] = coef((v * v).mul_add(45.0, 67.0).mul_add(5.0, c(0x44ac_bc36)), w),
            4 => self.set_focus(),
            5 => {
                self.g.tgt[G_DEF] = db(v.mul_add(40.0, -20.0));
                self.g.tgt[G_COMP] = db(v * -6.0);
            }
            6 => self.g.tgt[G_TRIM] = v * c(0x3ee4_dd2f),
            7 => {
                self.mixg.tgt[0] = v;
                self.mixg.tgt[1] = 1.0 - v;
                self.mixg.dirty = true;
            }
            _ => {}
        }
        if idx != 7 {
            self.g.dirty = true;
        }
        if self.suspended {
            self.g.snap();
        }
    }

    /// `onSetSampleRate(sr, 128)` (§4.4).
    fn set_sample_rate(&mut self, sr: f32) {
        let q = (sr as i32 as u32).wrapping_mul(0xb7a3_2847).wrapping_add(0x0057_6180).rotate_right(7);
        let fs = if q < 0x0001_5d87 { 192_000.0 } else { 176_400.0 };
        let blk = BLOCK as f32;
        let k = ((fs * blk) / sr) as i32;
        let ticks = ((fs * c(0x3ca3_d70a)) / k as f32 + 0.5) as i32;
        let ticks = ticks.max(1);
        self.sr = sr;
        self.fs = fs;
        self.g.ticks = ticks;
        self.g.inv = 1.0 / (ticks * k) as f32;
        let mt = ((sr * c(0x3ca3_d70a)) / blk + 0.5) as i32;
        let mt = mt.max(1);
        self.mixg.ticks = mt;
        self.mixg.inv = 1.0 / (mt * BLOCK as i32) as f32;
        let ratio = (fs / sr).round();
        if fs == sr {
            self.up = None;
            self.down = None;
        } else {
            let r = ratio as usize;
            self.up = Some([Up::new(r), Up::new(r)]);
            self.down = Some([Down::new(r), Down::new(r)]);
            let n = ((ratio * blk).ceil() as usize).max(1);
            self.wl = vec![0.0; n];
            self.wr = vec![0.0; n];
        }
        self.set_filters();
        self.set_focus();
        self.g.dirty = true;
        if self.suspended {
            self.g.snap();
        }
    }

    fn resume(&mut self) {
        self.g.snap();
        self.suspended = false;
    }

    fn control(&mut self) {
        self.mixg.tick();
        self.g.tick();
    }

    /// One channel's sample at the oversampled rate (§4.5).
    fn sample(&mut self, ch: usize, x: f32) -> f32 {
        let g = &self.g.cur;
        let s = self.bass[ch];
        let d = x - s;
        let a = (x - s).abs();
        self.bass[ch] = d.mul_add(g[G_BASS], s);
        let e = self.fenv;
        let a = a - e;
        let k = if a > 0.0 { g[G_FATT] } else { g[G_FREL] };
        let e = a.mul_add(k, e);
        self.fenv = e;
        let d = e.mul_add(g[G_FOCUS], d);
        let t = (d * g[G_SAT]).mul_add(8.0, 128.0);
        let n = t as i32;
        let f = t - n as f32;
        let i0 = n.clamp(0, 255) as usize;
        let i1 = ((if n < 0 { -1 } else { n }).min(254) + 1) as usize;
        let (y0, y1) = (f64::from_bits(SHAPE[i0]), f64::from_bits(SHAPE[i1]));
        let y = (y1 - y0).mul_add(f as f64, y0) as f32;
        let sm = (y - self.smooth[ch]).mul_add(g[G_SMOOTH], self.smooth[ch]);
        self.smooth[ch] = sm;
        let lp = (sm - self.lp[ch]).mul_add(g[G_TONE], self.lp[ch]);
        self.lp[ch] = lp;
        let hp = lp - self.dc[ch];
        self.dc[ch] = hp.mul_add(self.dc_c, self.dc[ch]);
        let dl = (x - self.dry_lp[ch]).mul_add(g[G_TONE], self.dry_lp[ch]);
        self.dry_lp[ch] = dl;
        dl.mul_add(g[G_DEF], hp) * g[G_COMP] * g[G_TRIM]
    }

    /// `processAudio` on one chunk (at most 128 samples): keep the dry input, convert up,
    /// shape, convert down, then glide-mix wet and dry.
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n = l.len();
        let mut dry = [(0f32, 0f32); BLOCK];
        for i in 0..n {
            dry[i] = (l[i], r[i]);
        }
        if let (Some(up), Some(_)) = (&self.up, &self.down) {
            let m = n * (up[0].h.len() + 1);
            let (mut wl, mut wr) = (std::mem::take(&mut self.wl), std::mem::take(&mut self.wr));
            if let Some(up) = &mut self.up {
                up[0].run(l, &mut wl[..m]);
                up[1].run(r, &mut wr[..m]);
            }
            for i in 0..m {
                self.g.step();
                wl[i] = self.sample(0, wl[i]);
                wr[i] = self.sample(1, wr[i]);
            }
            if let Some(down) = &mut self.down {
                down[0].run(&wl[..m], l);
                down[1].run(&wr[..m], r);
            }
            self.wl = wl;
            self.wr = wr;
        } else {
            for i in 0..n {
                self.g.step();
                l[i] = self.sample(0, l[i]);
                r[i] = self.sample(1, r[i]);
            }
        }
        for (i, &(dl, dr)) in dry[..n].iter().enumerate() {
            self.mixg.step();
            let (wet, dryg) = (self.mixg.cur[0], self.mixg.cur[1]);
            l[i] = l[i].mul_add(wet, dl * dryg);
            r[i] = r[i].mul_add(wet, dr * dryg);
        }
    }
}

/// `AnalogDistortion::s_fWaveShaperData`: the waveshaper's 256 points (`f64`), input
/// `128 + 8·x`, odd-symmetric around entry 128 (= 0).
static SHAPE: [u64; 256] = [
    0xbfe5517fffbf9353,
    0xbfe5517fffbf9353,
    0xbfe54d4000316462,
    0xbfe548bfffdfc9aa,
    0xbfe5444000179f50,
    0xbfe53f800015798f,
    0xbfe53b3ffffdda3f,
    0xbfe536ffffe63aee,
    0xbfe5330000089706,
    0xbfe52e8000406cad,
    0xbfe5293fffca501b,
    0xbfe523ffffdda3e8,
    0xbfe51ebffff0f7b6,
    0xbfe519c0003e46eb,
    0xbfe51500003c212a,
    0xbfe510400039fb68,
    0xbfe50b800037d5a7,
    0xbfe5067ffffbb47d,
    0xbfe500ffffd50ce2,
    0xbfe4fb800037d5a7,
    0xbfe4f5bfffd732a4,
    0xbfe4f0400039fb68,
    0xbfe4eaffffc3ded6,
    0xbfe4e5bfffd732a4,
    0xbfe4e0c0002481d9,
    0xbfe4db800037d5a7,
    0xbfe4d63fffc1b915,
    0xbfe4d07fffea8671,
    0xbfe4ca7fffd95865,
    0xbfe4c44000179f50,
    0xbfe4bdffffcc75dc,
    0xbfe4b7c0000abcc7,
    0xbfe4b1bffff98ebc,
    0xbfe4abbfffe860b0,
    0xbfe4a60000112e0c,
    0xbfe4a07fffea8671,
    0xbfe49a7fffd95865,
    0xbfe4947fffc82a59,
    0xbfe48e4000067144,
    0xbfe4877fffd0c15f,
    0xbfe4807fffea8671,
    0xbfe4793fffca501b,
    0xbfe4723fffe4152d,
    0xbfe46affffc3ded6,
    0xbfe463ffffdda3e8,
    0xbfe45d4000316462,
    0xbfe4567ffffbb47d,
    0xbfe4500000000000,
    0xbfe4493fffca501b,
    0xbfe44280001e1095,
    0xbfe43b3ffffdda3f,
    0xbfe433c0002d18df,
    0xbfe42bbfffe860b0,
    0xbfe4237ffff31d77,
    0xbfe41affffc3ded6,
    0xbfe4123fffe4152d,
    0xbfe4098000044b83,
    0xbfe400ffffd50ce2,
    0xbfe3f880002f3ea1,
    0xbfe3f0400039fb68,
    0xbfe3e8000044b830,
    0xbfe3e00000000000,
    0xbfe3d7c0000abcc7,
    0xbfe3cf00002af31e,
    0xbfe3c63fffc1b915,
    0xbfe3bcbfffbd6d92,
    0xbfe3b30000089706,
    0xbfe3a880002f3ea1,
    0xbfe39dc0001bead3,
    0xbfe39280001e1095,
    0xbfe3874000203656,
    0xbfe37bbfffe860b0,
    0xbfe3707fffea8671,
    0xbfe3653fffecac33,
    0xbfe35a400028cd5c,
    0xbfe34f800015798f,
    0xbfe3447fffc82a59,
    0xbfe3393fffca501b,
    0xbfe32dc0001bead3,
    0xbfe3217fffbf9353,
    0xbfe314c0000225c1,
    0xbfe3074000203656,
    0xbfe2f8bfffdfc9aa,
    0xbfe2e98000044b83,
    0xbfe2d98000044b83,
    0xbfe2c9000019c512,
    0xbfe2b8000044b830,
    0xbfe2a6ffffe63aee,
    0xbfe295bfffd732a4,
    0xbfe284c0000225c1,
    0xbfe2737ffff31d77,
    0xbfe26280001e1095,
    0xbfe250ffffd50ce2,
    0xbfe23f00002af31e,
    0xbfe22c3fffd2e721,
    0xbfe2183ffff54339,
    0xbfe20280001e1095,
    0xbfe1eaffffc3ded6,
    0xbfe1d17fffbf9353,
    0xbfe1b5bfffd732a4,
    0xbfe198000044b830,
    0xbfe1783ffff54339,
    0xbfe156c00035afe5,
    0xbfe133ffffdda3e8,
    0xbfe1100000000000,
    0xbfe0eb3ffffdda3f,
    0xbfe0c53fffecac33,
    0xbfe09dffffcc75dc,
    0xbfe0747fffc82a59,
    0xbfe047c0000abcc7,
    0xbfe0163fffc1b915,
    0xbfdfbb800037d5a7,
    0xbfdf377fffd0c15f,
    0xbfde9b800037d5a7,
    0xbfdde0ffffd50ce2,
    0xbfdd01ffffaa19c4,
    0xbfdbf8000044b830,
    0xbfdabd7fffe1ef6b,
    0xbfd94dffffcc75dc,
    0xbfd7a60000112e0c,
    0xbfd5c37ffff31d77,
    0xbfd3a680008524dc,
    0xbfd15100005e7d41,
    0xbfcd8d00010a49b9,
    0xbfc819ffffeed1f4,
    0xbfc258ffff06e453,
    0xbfb8b60001ad7f2a,
    0xbfa8d7fffe1ef6b3,
    0x0000000000000000,
    0x3fa8c0000225c17d,
    0x3fb887fffea86712,
    0x3fc224ffff29406b,
    0x3fc7c00000000000,
    0x3fcd0500003c212a,
    0x3fd0f30000089706,
    0x3fd32c7fff83722a,
    0x3fd52cfffff768fa,
    0x3fd6f380007c8dd6,
    0x3fd880800073f6d0,
    0x3fd9d700006fab4d,
    0x3fdafb800037d5a7,
    0x3fdbf27fff94a036,
    0x3fdcc1ffffaa19c4,
    0x3fdd70000089705f,
    0x3fde02ffff7f26a7,
    0x3fde80800073f6d0,
    0x3fdeed80006b5fca,
    0x3fdf4effffa182bf,
    0x3fdfa780005a31bf,
    0x3fdffaffffc3ded6,
    0x3fe0253fffecac33,
    0x3fe04bbfffe860b0,
    0x3fe07140000f084a,
    0x3fe095800026a79b,
    0x3fe0b83ffff54339,
    0x3fe0d9c0003e46eb,
    0x3fe0f93fffca501b,
    0x3fe116ffffe63aee,
    0x3fe13280001e1095,
    0x3fe14c0000225c18,
    0x3fe1637ffff31d77,
    0x3fe179000019c512,
    0x3fe18d7fffe1ef6b,
    0x3fe1a0c0002481d9,
    0x3fe1b30000089706,
    0x3fe1c500003c212a,
    0x3fe1d6c00035afe5,
    0x3fe1e880002f3ea1,
    0x3fe1fa400028cd5c,
    0x3fe20bbfffe860b0,
    0x3fe21d4000316462,
    0x3fe22e8000406cad,
    0x3fe23f3fffdb7e27,
    0x3fe24f3fffdb7e27,
    0x3fe25ebffff0f7b6,
    0x3fe26d4000316462,
    0x3fe27b3ffffdda3f,
    0x3fe2883ffff54339,
    0x3fe294c0000225c1,
    0x3fe2a07fffea8671,
    0x3fe2ac3fffd2e721,
    0x3fe2b77fffd0c15f,
    0x3fe2c30000089706,
    0x3fe2ce8000406cad,
    0x3fe2d9ffffeed1f4,
    0x3fe2e5bfffd732a4,
    0x3fe2f140000f084a,
    0x3fe2fcbfffbd6d92,
    0x3fe3083ffff54339,
    0x3fe313400042926e,
    0x3fe31dc0001bead3,
    0x3fe328000044b830,
    0x3fe3317fffbf9353,
    0x3fe33ac0001353cd,
    0x3fe343c0002d18df,
    0x3fe34c3fffd2e721,
    0x3fe35500003c212a,
    0x3fe35d7fffe1ef6b,
    0x3fe3660000112e0c,
    0x3fe36ebffff0f7b6,
    0x3fe377c0000abcc7,
    0x3fe3807fffea8671,
    0x3fe3898000044b83,
    0x3fe3923fffe4152d,
    0x3fe39ac0001353cd,
    0x3fe3a2bfffce9b9e,
    0x3fe3aa7fffd95865,
    0x3fe3b20000338a24,
    0x3fe3b93fffca501b,
    0x3fe3c0400039fb68,
    0x3fe3c74000203656,
    0x3fe3ce4000067144,
    0x3fe3d53fffecac33,
    0x3fe3dc80000ce289,
    0x3fe3e3ffffdda3e8,
    0x3fe3eb3ffffdda3f,
    0x3fe3f2bfffce9b9e,
    0x3fe3f9c0003e46eb,
    0x3fe400c0002481d9,
    0x3fe4074000203656,
    0x3fe40dc0001bead3,
    0x3fe413c0002d18df,
    0x3fe419c0003e46eb,
    0x3fe41fbfffc60498,
    0x3fe4260000112e0c,
    0x3fe42c3fffd2e721,
    0x3fe43280001e1095,
    0x3fe439000019c512,
    0x3fe43f3fffdb7e27,
    0x3fe445800026a79b,
    0x3fe44b800037d5a7,
    0x3fe45140000f084a,
    0x3fe456c00035afe5,
    0x3fe45c0000225c18,
    0x3fe4617fffbf9353,
    0x3fe466ffffe63aee,
    0x3fe46cbfffbd6d92,
    0x3fe47280001e1095,
    0x3fe4783ffff54339,
    0x3fe47dffffcc75dc,
    0x3fe483400042926e,
    0x3fe48880002f3ea1,
    0x3fe48d7fffe1ef6b,
    0x3fe49280001e1095,
    0x3fe497c0000abcc7,
    0x3fe49cfffff768fa,
    0x3fe4a280001e1095,
    0x3fe4a7c0000abcc7,
    0x3fe4ac80000ce289,
    0x3fe4b0ffffd50ce2,
    0x3fe4b53fffecac33,
    0x3fe4b9c0003e46eb,
    0x3fe4be8000406cad,
    0x3fe4c3400042926e,
    0x3fe4c77fffd0c15f,
    0x3fe4c77fffd0c15f,
];

// ---------------------------------------------------------------------------------------
// The processor (§2)
// ---------------------------------------------------------------------------------------

/// Maschine's Distortion: `RT::MullhollandDriveProcessor` with both cores.
pub struct Distortion {
    sr: f32,
    mode: u32,
    mh: Box<Mulholland>,
    an: Box<Analog>,
    /// Samples left until the Analog core's next control tick (processor +0xb8).
    left: usize,
    values: Option<[f32; 15]>,
}

impl Distortion {
    pub fn new(sr: f64) -> Self {
        Distortion { sr: sr as f32, mode: 0, mh: Mulholland::new(44100.0), an: Analog::new(), left: 0, values: None }
    }

    /// `processFloatParameter` / `processBoolParameter` / `setMode` for `PARAMS[k]`.
    fn apply(&mut self, k: usize, v: f32) {
        match k {
            0 => self.mode = v as u32,
            1 => self.mh.set_drive(v),
            2 => self.mh.set_color(v),
            3 => self.mh.set_fb(v),
            4 => self.mh.set_tone(v),
            5 => self.mh.set_tone_mod(v),
            6 => self.mh.set_gate(v != 0.0),
            7 => self.mh.set_release(v),
            8 => self.mh.set_mix(v),
            9 => self.an.set_param(0, v),
            10 => self.an.set_param(1, v),
            11 => self.an.set_param(3, v),
            12 => self.an.set_param(4, if v != 0.0 { 1.0 } else { 0.0 }),
            13 => self.an.set_param(5, v),
            14 => self.an.set_param(7, v),
            _ => {}
        }
    }
}

impl Effect for Distortion {
    fn set(&mut self, params: &[(String, f64)]) {
        let new = PARAMS.map(|(name, def)| param(params, name, def as f64) as f32);
        match self.values {
            None => {
                // The module's construction (each default reaches the processor), the
                // project's values, the host's sample rate, then `resume`.
                for (k, (_, def)) in PARAMS.iter().enumerate().skip(1) {
                    self.apply(k, *def);
                }
                for (k, v) in new.iter().enumerate() {
                    self.apply(k, *v);
                }
                self.mh.set_sample_rate(self.sr);
                self.an.set_sample_rate(self.sr);
                self.an.resume();
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

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n = l.len().min(r.len());
        let (l, r) = (&mut l[..n], &mut r[..n]);
        match self.mode {
            0 => self.mh.process(l, r),
            1 => {
                let mut i = 0;
                while i < n {
                    if self.left == 0 {
                        self.an.control();
                        self.left = BLOCK;
                    }
                    let m = (n - i).min(self.left);
                    self.an.process(&mut l[i..i + m], &mut r[i..i + m]);
                    self.left -= m;
                    i += m;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(kv: &[(&str, f64)]) -> Vec<(String, f64)> {
        kv.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn run(fx: &mut Distortion, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut a, mut b) = (l.to_vec(), r.to_vec());
        for (x, y) in a.chunks_mut(300).zip(b.chunks_mut(300)) {
            fx.process(x, y);
        }
        (a, b)
    }

    fn tone(n: usize, amp: f32) -> Vec<f32> {
        (0..n).map(|i| (i as f32 * 0.0573).sin() * amp).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn converter_phases() {
        // The ½ phase is symmetric, and each phase sums to ≈ 1 (unity DC gain).
        let h = src_phase(0.5);
        for k in 0..16 {
            assert_eq!(h[k], h[15 - k]);
        }
        for t in [0.25, 0.5, 0.75] {
            let s: f32 = src_phase(t).iter().sum();
            assert!((s - 1.0).abs() < 0.01, "{t}: {s}");
        }
    }

    #[test]
    fn converters_round_trip_delay() {
        // Up then down ×4 is a near-identity with a fixed delay.
        let (mut up, mut down) = (Up::new(4), Down::new(4));
        let x = tone(1024, 0.5);
        let mut hi = vec![0f32; 4096];
        let mut y = vec![0f32; 1024];
        up.run(&x, &mut hi);
        down.run(&hi, &mut y);
        let lag = (0..40).min_by_key(|&d| {
            let e: f32 = (200..900).map(|i| (y[i + d] - x[i]).powi(2)).sum();
            (e * 1e6) as i64
        });
        let d = lag.unwrap();
        let e = rms(&(200..900).map(|i| y[i + d] - x[i]).collect::<Vec<_>>());
        assert!(e < 1e-3, "lag {d}: {e}");
    }

    #[test]
    fn both_modes_are_finite_and_bounded() {
        for mode in [0.0, 1.0] {
            for sr in [44100.0, 48000.0, 96000.0] {
                let mut fx = Distortion::new(sr);
                fx.set(&params(&[("mode", mode), ("drive", 1.0), ("feedback", 1.0), ("release", 0.0), ("analog_saturation", 1.0)]));
                let x = tone(20000, 1.0);
                let (l, r) = run(&mut fx, &x, &x);
                assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 8.0), "mode {mode} sr {sr}");
                // Silence afterwards decays (the gate closes on the self-oscillating
                // feedback) without denormal blow-up.
                let (l, _) = run(&mut fx, &vec![0.0; 96000], &vec![0.0; 96000]);
                let pk = l[95000..].iter().fold(0f32, |m, v| m.max(v.abs()));
                assert!(pk < 1e-6, "mode {mode} sr {sr}: {pk} genv {}", fx.mh.genv);
            }
        }
    }

    #[test]
    fn mix_zero_is_dry() {
        // Mulholland: Mix 0 → dry gain (1 − 0)(2 − 1) = 1, wet 0. Analog: Mix 0 after its glide.
        let x = tone(30000, 0.3);
        let mut fx = Distortion::new(48000.0);
        fx.set(&params(&[("mode", 0.0), ("mix", 0.0)]));
        let (l, _) = run(&mut fx, &x, &x);
        assert!(l.iter().zip(&x).all(|(a, b)| a == b));
        let mut fx = Distortion::new(48000.0);
        fx.set(&params(&[("mode", 1.0), ("analog_mix", 0.0)]));
        let (l, _) = run(&mut fx, &x, &x);
        assert!(l[5000..].iter().zip(&x[5000..]).all(|(a, b)| a == b));
    }

    #[test]
    fn stereo_channels_are_independent_in_analog_path() {
        // A silent right channel stays near silent while the left is driven (only the
        // shared focus envelope couples them, and Focus off removes it).
        let mut fx = Distortion::new(44100.0);
        fx.set(&params(&[("mode", 1.0), ("analog_focus", 1.0)]));
        let x = tone(20000, 0.5);
        let (l, r) = run(&mut fx, &x, &vec![0.0; 20000]);
        assert!(rms(&l) > 0.05);
        assert!(rms(&r) < 1e-3, "{}", rms(&r));
    }

    #[test]
    fn version0_projects_are_mulholland() {
        let mut fx = Distortion::new(44100.0);
        fx.set(&params(&[("drive", 0.5)]));
        assert_eq!(fx.mode, 0);
    }

    #[test]
    fn parameter_changes_keep_state() {
        let mut fx = Distortion::new(48000.0);
        fx.set(&params(&[("mode", 0.0)]));
        let x = tone(4800, 0.5);
        run(&mut fx, &x, &x);
        fx.set(&params(&[("mode", 0.0), ("drive", 0.9), ("color", 0.8)]));
        let (l, _) = run(&mut fx, &x, &x);
        // No reset: output continues (no silence gap at the change).
        assert!(rms(&l[..100]) > 1e-3);
    }
}
