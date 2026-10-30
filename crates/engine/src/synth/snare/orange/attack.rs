//! `NI::DSP::AttackCore<FluffyOsc on, white noise, tri, parabolic saturator, …>`: the
//! attack ("impact") layer of Iron. Three sources summed with fixed gains: a short
//! half-cycle "fluffy" click, the main oscillator (a phase-modulated triangle with
//! four sine phase oscillators, a pitch envelope, a parabolic saturator and a one-pole
//! low-pass) and a band-passed white noise. The click plays first; the oscillator and
//! the noise start when it ends. [exact]

use super::env::{DB, Env, MS, rate, release_mul};
use crate::fx::exp_e;

const fn f(b: u32) -> f32 {
    f32::from_bits(b)
}

const TWO_PI: f32 = f(0x40c9_0fdb);
const PI: f32 = f(0x4049_0fdb);
/// `x·5 + 1381.8816`: semitones above C−1 → table units for a frequency in Hz.
pub(crate) const HZ0: f32 = f(0x44ac_bc36);
/// 2^−31 (`NoiseGen` scale).
pub(crate) const RND: f32 = f(0x3000_009d);
/// The LCG of `NoiseGen` / `MainOscAttack`.
#[inline]
pub(crate) fn lcg(s: i32) -> i32 {
    s.wrapping_mul(0x41c6_4e6d).wrapping_add(12345)
}

/// The velocity curve with two shapes (`lo`, `hi`, `s1`, `s2`), clamped to 0..1.
pub(crate) fn curve2(v: f32, lo: f32, hi: f32, s1: f32, s2: f32) -> f32 {
    let mut t = ramp(v, lo, hi);
    t *= (-t.abs()).mul_add(s1, s1 + 1.0);
    t *= (-t.abs()).mul_add(s2, s2 + 1.0);
    clamp01(t)
}

/// The velocity curve with a shape and an amount (`lo`, `hi`, `sh`, `amt`).
pub(crate) fn curve_amt(v: f32, lo: f32, hi: f32, sh: f32, amt: f32) -> f32 {
    let mut t = ramp(v, lo, hi);
    t *= (-t.abs()).mul_add(sh, sh + 1.0);
    let a = 1.0 - t;
    let k = if amt + amt > 1.0 { 1.0 } else { amt + amt };
    let a = (1.0 - a).mul_add(k, a);
    let k2 = amt.mul_add(2.0, -k);
    clamp01((t - a).mul_add(k2, a))
}

/// `(clamp(v, lo, hi') − lo) / max(hi' − lo, 1e-7)`, hi' = hi > lo ? hi : lo + 0.002.
#[inline]
fn ramp(v: f32, lo: f32, hi: f32) -> f32 {
    let hi = if hi > lo { hi } else { lo + f(0x3b03_126f) };
    let x = if lo > v { lo } else { v };
    let x = if x > hi { hi } else { x };
    let d = hi - lo;
    let d = if d < f(0x33d6_bf95) { f(0x33d6_bf95) } else { d };
    (x - lo) / d
}

#[inline]
pub(crate) fn clamp01(x: f32) -> f32 {
    let x = if x < 0.0 { 0.0 } else { x };
    if x > 1.0 { 1.0 } else { x }
}

#[inline]
pub(crate) fn clamp11(x: f32) -> f32 {
    let x = if x < -1.0 { -1.0 } else { x };
    if x > 1.0 { 1.0 } else { x }
}

/// `x + x³/3 + 2x⁵/15`: Maschine's tan(x) for small x.
#[inline]
pub(crate) fn tan3(w: f32) -> f32 {
    let w2 = w * w;
    let w3 = w * w2;
    let w5 = w2 * w3;
    w + w5.mul_add(f(0x3e08_8886), w3 * f(0x3eaa_aaaa))
}

/// `NI::DSP::UTIL::PhaseOsc<sine>` (0xc0 bytes): a sine with its own envelope.
#[derive(Clone, Copy)]
pub(crate) struct PhaseOsc {
    /// +0x00 frequency (Hz), +0x04 release time, +0x08 amplitude.
    pub freq: f32,
    pub release: f32,
    pub amp: f32,
    /// +0x0c on, +0x0d (unused here), +0x0e restart the phase at note-on.
    pub on: bool,
    pub b0d: bool,
    pub reset_phase: bool,
    /// +0x10 attack time of the envelope.
    pub pitch: f32,
    /// +0x18.
    pub env: Env,
    /// +0x98 phase (−0.5..0.5), +0x9c last sine.
    pub phase: f32,
    pub sine: f32,
    /// +0xa0 / +0xa4.
    pub sr: f32,
    pub inv_sr: f32,
    /// +0xa8 (written, unused).
    pub fa8: f32,
    /// +0xac pitch modulation (ratio − 1), +0xb0 amplitude modulation.
    pub pitch_mod: f32,
    pub amp_mod: f32,
    /// +0xb4 output gain, +0xb8 phase offset, +0xbc frequency ratio.
    pub gain: f32,
    pub phase_off: f32,
    pub ratio: f32,
}

impl PhaseOsc {
    const fn new() -> PhaseOsc {
        PhaseOsc {
            freq: 0.0,
            release: 0.0,
            amp: 0.0,
            on: false,
            b0d: false,
            reset_phase: false,
            pitch: 0.0,
            env: Env::new(),
            phase: 0.0,
            sine: 0.0,
            sr: 1.0,
            inv_sr: 1.0,
            fa8: 1.0,
            pitch_mod: 0.0,
            amp_mod: 0.0,
            gain: 1.0,
            phase_off: 0.0,
            ratio: 1.0,
        }
    }

    /// `PhaseOsc::init(sr, attack, amp, restart)`.
    fn init(&mut self, sr: f32, pitch: f32, amp: f32, reset: bool) {
        if !(1.0..=500_000.0).contains(&sr) || !(-24.0..=40.0).contains(&pitch) || !(0.0..=1.0).contains(&amp) {
            return;
        }
        self.sr = sr;
        self.inv_sr = 1.0 / sr;
        self.fa8 = 0.0;
        self.env.set_sample_rate(sr);
        let inv = self.env.inv_sr;
        self.env.hold_len = f64::from((exp_e(602.0 + f(0x3d58_c000)) + MS) * self.env.sr);
        self.env.hold = 0.0;
        self.env.decay = 0.0;
        self.env.decay_rate = inv * f64::from(exp_e(1797.0 + f(0x3f72_7000)));
        self.env.sustain = 1.0;
        self.pitch = pitch;
        self.env.attack = pitch;
        self.env.attack_rate = f64::from(rate(pitch)) * inv;
        self.reset_phase = reset;
        self.env.release = self.release;
        self.env.release_mul = release_mul(inv, self.release);
    }

    #[inline]
    fn process(&mut self) -> f32 {
        if !self.on {
            return 0.0;
        }
        let inc = clamp11((self.pitch_mod + 1.0) * self.freq * self.ratio * self.inv_sr);
        let p = self.phase + inc;
        let ph = if p < 0.5 { p } else { p + -1.0 };
        let m = if inc < 0.01 { 0.01 } else { inc };
        let aa = if m > 0.03 { 0.02 } else { m + -0.01 };
        let x = self.phase_off + ph;
        let t = x - (x as i32) as f32;
        let y = t + -0.25;
        let r = if y < 0.0 { -0.5f64 } else { 0.5 } + f64::from(y);
        let y = y - (r as i32) as f32;
        let s = 0.5 - (y + y).abs();
        let s2 = s * s;
        let p1 = s2.mul_add(f(0x4010_fda0), f(0xc0a4_3f68));
        let p2 = p1.mul_add(s2, f(0x4049_0fd0));
        self.fa8 = inc * 100.0 * aa;
        let sine = s * p2;
        self.phase = ph;
        self.sine = sine;
        let a = clamp01(self.amp + self.amp_mod);
        let out = self.gain * a * sine;
        out * self.env.tick()
    }
}

/// `NI::DSP::UTIL::MainOscAttack<tri, parabolic, …>` (at AttackCore +0x08).
#[derive(Clone, Copy)]
pub(crate) struct MainOsc {
    /// +0x00 base frequency (Hz), +0x04 triangle skew (−1..1).
    pub freq: f32,
    pub skew: f32,
    /// +0x08 restart phases at note-on.
    pub restart: bool,
    /// +0x0c phase-osc amplitude (and random detune depth), +0x10 their attack time.
    pub osc_amp: f32,
    pub osc_attack: f32,
    /// +0x14 pitch-envelope depth, +0x18 its modulation.
    pub penv_amt: f32,
    pub penv_mod: f32,
    /// +0x1c..+0x28: the drive envelope's velocity curve (lo, hi, shape, amount).
    pub vc2: [f32; 4],
    /// +0x2c drive-envelope depth.
    pub drive_amt: f32,
    /// +0x30..+0x50: parameter copies (not read by the DSP).
    pub copies: [f32; 9],
    /// +0x54 phase coupling of osc 2 to osc 1.
    pub coupling: f32,
    /// +0x58 frequency scale, +0x5c frequency ratio (tune).
    pub fscale: f32,
    pub ratio: f32,
    /// +0x60/+0x64 low-pass cutoff range (semitones), +0x68..+0x74 its velocity curve.
    pub cut_lo: f32,
    pub cut_hi: f32,
    pub vc_cut: [f32; 4],
    /// +0x78..+0x84 the main velocity curve.
    pub vc1: [f32; 4],
    /// +0x88 amplitude, +0x108 pitch, +0x188 drive envelopes.
    pub env_amp: Env,
    pub env_pitch: Env,
    pub env_drive: Env,
    /// +0x208..+0x214 saturator drive, max(drive, 2e-12), twice that, 1/that.
    pub drive: [f32; 4],
    /// +0x218 low-pass state, +0x21c high-pass state, +0x220/+0x224 copies.
    pub lp: f32,
    pub hp: f32,
    pub lp2: f32,
    pub hp2: f32,
    /// +0x228 one-pole coefficient, +0x22c cutoff (Hz), +0x230 1/sr.
    pub coef: f32,
    pub cutoff: f32,
    pub cut_inv_sr: f32,
    /// +0x238 the four phase oscillators.
    pub osc: [PhaseOsc; 4],
    /// +0x538 velocity, +0x53c 1000/(2·sr), +0x540, +0x544.
    pub vel: f32,
    pub f53c: f32,
    pub f540: f32,
    pub f544: f32,
    /// +0x54c phase, +0x550 sr, +0x554 1/sr.
    pub phase: f32,
    pub sr: f32,
    pub inv_sr: f32,
    /// +0x558 anti-alias width, +0x55c base increment.
    pub width: f32,
    pub inc: f32,
    /// +0x560 last triangle, +0x564 random seed, +0x568 random detune.
    pub tri: f32,
    pub seed: i32,
    pub rnd: f32,
    /// +0x56c last cutoff pitch.
    pub last_cut: f32,
}

impl MainOsc {
    const fn new() -> MainOsc {
        let mut o = MainOsc {
            freq: 0.0,
            skew: 0.0,
            restart: false,
            osc_amp: 0.0,
            osc_attack: 0.0,
            penv_amt: 0.0,
            penv_mod: 0.0,
            vc2: [0.0, 1.0, 0.0, 0.0],
            drive_amt: 0.0,
            copies: [0.0; 9],
            coupling: 0.0,
            fscale: 1.0,
            ratio: 1.0,
            cut_lo: 0.0,
            cut_hi: 0.0,
            vc_cut: [0.0; 4],
            vc1: [0.0; 4],
            env_amp: Env::new(),
            env_pitch: Env::new(),
            env_drive: Env::new(),
            drive: [1.0, 1.0, 2.0, 1.0],
            lp: 0.0,
            hp: 0.0,
            lp2: 0.0,
            hp2: 0.0,
            coef: 0.0,
            cutoff: 1.0,
            cut_inv_sr: 1.0,
            osc: [PhaseOsc::new(); 4],
            vel: 0.0,
            f53c: 500.0,
            f540: 0.0,
            f544: 0.0,
            phase: 1.0,
            sr: 1.0,
            inv_sr: 1.0,
            width: 0.0,
            inc: 0.0,
            tri: 0.0,
            seed: 1,
            rnd: 0.0,
            last_cut: -1.0,
        };
        o.osc[3].gain = 1.0;
        o
    }

    /// `MainOscAttack::setSampleRate`.
    fn set_sample_rate(&mut self, sr: f32) {
        if !(1.0..=500_000.0).contains(&sr) {
            return;
        }
        self.sr = sr;
        let inv = 1.0 / sr;
        self.inv_sr = inv;
        self.f53c = 1000.0 / (sr + sr);
        self.f544 = 0.0;
        self.phase = 1.0;
        self.width = 0.0;
        self.inc = 0.0;
        for i in 0..4 {
            let (p, a, r) = (self.osc_attack, self.osc_amp, self.restart);
            self.osc[i].init(sr, p, a, r);
        }
        let d0 = exp_e(1797.0 + f(0x3f72_7000));
        for e in [&mut self.env_amp, &mut self.env_pitch, &mut self.env_drive] {
            e.set_sample_rate(sr);
            e.decay = 0.0;
            e.decay_rate = e.inv_sr * f64::from(d0);
            e.sustain = 1.0;
        }
        self.cut_inv_sr = inv;
        let old = self.cutoff;
        self.cutoff = -1.0;
        if old != -1.0 {
            self.cutoff = old;
            let c = inv * TWO_PI * old;
            self.coef = if c > 1.0 { 1.0 } else { c };
        }
        self.lp = 0.0;
        self.hp = 0.0;
        self.lp2 = 0.0;
        self.hp2 = 0.0;
        self.inc = clamp11(self.inv_sr * (self.freq * (self.ratio * self.fscale)));
        self.width = Self::aa_width(self.inc);
        self.skew = clamp11(self.skew);
        self.restart = (if self.restart { 1.0f32 } else { 0.0 }) == 1.0;
        let e = f64::from(rate(self.osc_attack));
        let (p, r) = (self.osc_attack, self.restart);
        for o in &mut self.osc {
            o.reset_phase = r;
            o.pitch = p;
            o.env.attack = p;
            o.env.attack_rate = e * o.env.inv_sr;
        }
    }

    /// `2·inc · min(1, (max(inc, 0.01) − 0.01)/0.02)`.
    #[inline]
    fn aa_width(inc: f32) -> f32 {
        let m = if inc < 0.01 { 0.01 } else { inc };
        let t = (m + -0.01) / 0.02;
        (inc + inc) * if m > 0.03 { 1.0 } else { t }
    }

    /// `MainOscAttack::setNoteOn(velocity)`.
    fn note_on(&mut self, vel: f32) {
        if !(0.0..=1.0).contains(&vel) {
            return;
        }
        self.vel = vel;
        let [lo, hi, s1, s2] = self.vc1;
        let v = curve2(vel, lo, hi, s1, s2);
        self.env_amp.note_on(v);
        self.env_pitch.note_on(v);
        let [lo, hi, sh, amt] = self.vc2;
        self.env_drive.note_on(curve_amt(v, lo, hi, sh, amt));
        self.seed = lcg(self.seed);
        self.rnd = self.osc_amp * self.osc_amp * ((self.seed as f32) * RND);
        for o in &mut self.osc {
            o.pitch_mod = self.rnd;
            o.env.note_on(v);
            if o.reset_phase {
                o.phase = 0.0;
            }
        }
        if self.restart {
            self.phase = 1.0;
            self.f540 = 0.0;
            self.f544 = self.tri;
        }
    }

    #[inline]
    fn process(&mut self) -> f32 {
        let base = self.freq * self.fscale * self.ratio;
        let pe = self.penv_amt + self.penv_mod;
        let pe = if pe < 0.0 { 0.0 } else { pe };
        let pe = if pe > 5.0 { 5.0 } else { pe };
        let pv = self.env_pitch.tick();
        let inc = clamp11(self.inv_sr * (base * pe.mul_add(pv, 1.0)));
        let w = Self::aa_width(self.inc);
        let p = self.phase + inc;
        let ph = if p < 0.5 { p } else { p + -1.0 };
        self.phase = ph;
        let coupling = self.coupling;
        let mut sum = self.osc[0].process() + 0.0;
        if coupling > 0.0 {
            self.osc[1].phase_off = self.osc[0].sine * self.coupling;
        }
        sum += self.osc[1].process();
        sum += self.osc[2].process();
        let x = ph + (sum + self.osc[3].process());
        let r = if x < 0.0 { -0.5f64 } else { 0.5 } + f64::from(x);
        let x = x - (r as i32) as f32;
        let x = x + x;
        let k = (self.skew + 1.0) * 0.5;
        let k = (0.5 - k).mul_add(w, k);
        let a = x.abs();
        let (num, den) = if a > k { (1.0 - a, 1.0 - k) } else { (a, k) };
        let den = if den < f(0x3586_37bd) { f(0x3586_37bd) } else { den };
        let q = num / den;
        let w2 = if w + w > 1.0 { 1.0 } else { w + w };
        let g = (-q.abs()).mul_add(w2, w2 + 1.0);
        let tri = if x > 0.0 { q * g } else { -(q * g) };
        self.tri = tri;
        let dv = self.env_drive.tick();
        let drive = (dv * f(0xbf4c_cccc)).mul_add(dv * self.drive_amt, f(0x3f66_6666));
        let m = if drive < f(0x2c0c_bccc) { f(0x2c0c_bccc) } else { drive };
        let lim = m + m;
        let inv_m = 1.0 / m;
        self.drive = [drive, m, lim, inv_m];
        let av = self.env_amp.tick();
        let t = curve2(self.vel, self.vc_cut[0], self.vc_cut[1], self.vc_cut[2], self.vc_cut[3]);
        let cut = (self.cut_hi - self.cut_lo).mul_add(t, self.cut_lo);
        if cut != self.last_cut {
            let hz = exp_e(cut.mul_add(5.0, HZ0));
            if self.cutoff != hz {
                self.cutoff = hz;
                let c = hz * (self.cut_inv_sr * TWO_PI);
                self.coef = if c > 1.0 { 1.0 } else { c };
            }
            self.last_cut = cut;
        }
        let y = if tri < -lim { -lim } else { tri };
        let y = if y > lim { lim } else { y };
        let k = (y.abs() * -0.25).mul_add(inv_m, 1.0);
        let y = y * k;
        let s = (1.0 / drive) * y * av;
        let lp = self.coef.mul_add(self.hp, self.lp);
        self.lp2 = lp;
        let hp = s - lp;
        self.hp2 = hp;
        self.lp = lp;
        self.hp = hp;
        lp
    }
}

/// `NI::DSP::UTIL::FluffyOsc` (at AttackCore +0x578): one shaped half cycle.
#[derive(Clone, Copy)]
pub(crate) struct Fluffy {
    /// +0x00..+0x0c velocity curve (lo, hi, shape, amount).
    pub vc: [f32; 4],
    /// +0x10 2 = half cycle (else full), +0x14 phase ceiling.
    pub mode: f32,
    pub ceil: f32,
    /// +0x18 gain, +0x1c frequency, +0x20 shape, +0x24 level.
    pub gain: f32,
    pub freq: f32,
    pub shape: f32,
    pub level: f32,
    /// +0x28 phase, +0x2c sr, +0x30 1/sr, +0x34 increment.
    pub phase: f32,
    pub sr: f32,
    pub inv_sr: f32,
    pub inc: f32,
    /// +0x38 sounding, +0x3c count, +0x40 length.
    pub on: bool,
    pub count: u32,
    pub len: u32,
}

impl Fluffy {
    const fn new() -> Fluffy {
        Fluffy {
            vc: [0.0, 1.0, 0.0, 0.0],
            mode: 0.0,
            ceil: 1.0,
            gain: 0.0,
            freq: 20.0,
            shape: 0.0,
            level: 0.0,
            phase: 0.0,
            sr: 1.0,
            inv_sr: 1.0,
            inc: 0.0,
            on: false,
            count: 0,
            len: 0,
        }
    }

    fn set_sample_rate(&mut self, sr: f32) {
        if !(1.0..=500_000.0).contains(&sr) {
            return;
        }
        let inv = 1.0 / sr;
        self.sr = sr;
        self.inv_sr = inv;
        self.inc = clamp11((inv + inv) * self.freq);
        let m = if self.mode == 1.0 { 2.0 } else { 1.0 };
        self.len = trunc_up((1.0 / (self.freq * m)) * sr);
        self.level = 0.0;
    }

    fn note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        self.phase = -1.0;
        self.count = 0;
        self.on = true;
        let [lo, hi, sh, amt] = self.vc;
        self.level = curve_amt(v, lo, hi, sh, amt);
    }

    #[inline]
    fn process(&mut self) -> f32 {
        let p = self.phase + self.inc;
        let p = if self.ceil < p { self.ceil } else { p };
        self.phase = p;
        if self.count >= self.len {
            self.on = false;
        }
        let s5 = self.shape + 1.0;
        let x = p * (-p.abs()).mul_add(self.shape, s5);
        let x = x * (-x.abs()).mul_add(self.shape, s5);
        let x = x * 0.5;
        let g = self.level * self.gain;
        let y = x * x.abs().mul_add(-16.0, 8.0);
        self.count += 1;
        g * y
    }
}

/// `(int)(x < 0 ? x : x + 1)` in double, as Maschine rounds a length up.
#[inline]
pub(crate) fn trunc_up(x: f32) -> u32 {
    let d = f64::from(x);
    (if x < 0.0 { d } else { d + 1.0 }) as i32 as u32
}

/// `NI::DSP::UTIL::ParallelNoise<white>` (at AttackCore +0x5c0): white noise through a
/// third-order band-pass and a second-order low-pass, with an envelope.
#[derive(Clone, Copy)]
pub(crate) struct Noise {
    /// +0x00 gain, +0x04 gain parameter, +0x08 band-pass frequency (Hz),
    /// +0x0c low-pass pitch, +0x10 low-pass frequency (Hz).
    pub gain: f32,
    pub gain_par: f32,
    pub bp_hz: f32,
    pub lp_pitch: f32,
    pub lp_hz: f32,
    /// +0x18 (a flag, unused here).
    pub b18: bool,
    /// +0x1c..+0x28 envelope parameter copies, +0x2c release base.
    pub env_pars: [f32; 4],
    pub release0: f32,
    /// +0x30..+0x3c velocity curve.
    pub vc: [f32; 4],
    /// +0x40 low-pass modulation, +0x4c release modulation, +0x50 gain modulation.
    pub lp_mod: f32,
    pub rel_mod: f32,
    pub gain_mod: f32,
    /// `NoiseGen` (+0x54): reseed at note-on, seed at +0x5c, scale (+0xe4), state (+0xec).
    pub reseed: bool,
    pub seed0: i32,
    pub scale: f32,
    pub seed: i32,
    /// +0x118.
    pub env: Env,
    /// +0x198..: band-pass states (hp, bp, lp) and copies.
    pub s: [f32; 6],
    /// +0x1b0 g, +0x1b4 k, +0x1b8 g(g+k), +0x1bc frequency, +0x1c0 1/(1+g(g+k)).
    pub g1: f32,
    pub k1: f32,
    pub gk1: f32,
    pub f1: f32,
    pub h1: f32,
    pub flag1: bool,
    pub sr1: f32,
    pub inv1: f32,
    /// +0x1d0 previous input, +0x1d4/+0x1d8 states, +0x1dc..+0x1e4 outputs.
    pub x1: f32,
    pub z1: f32,
    pub z2: f32,
    pub o: [f32; 3],
    /// +0x1e8 g, +0x1ec k, +0x1f0 coefficient, +0x1f4 frequency.
    pub g2: f32,
    pub k2: f32,
    pub c2: f32,
    pub f2: f32,
    pub flag2: bool,
    pub sr2: f32,
    pub inv2: f32,
    pub sr: f32,
    pub inv_sr: f32,
    pub block: i32,
    pub vel: f32,
}

impl Noise {
    const fn new() -> Noise {
        Noise {
            gain: 0.0,
            gain_par: 0.0,
            bp_hz: 0.0,
            lp_pitch: 0.0,
            lp_hz: 0.0,
            b18: false,
            env_pars: [0.0; 4],
            release0: 0.0,
            vc: [0.0; 4],
            lp_mod: 0.0,
            rel_mod: 0.0,
            gain_mod: 0.0,
            reseed: false,
            seed0: 2000,
            scale: RND,
            seed: 0,
            env: Env::new(),
            s: [0.0; 6],
            g1: 0.0,
            k1: 2.0,
            gk1: 0.0,
            f1: 1.0,
            h1: 0.0,
            flag1: false,
            sr1: 1.0,
            inv1: 1.0,
            x1: 0.0,
            z1: 0.0,
            z2: 0.0,
            o: [0.0; 3],
            g2: 0.0,
            k2: 2.0,
            c2: 0.0,
            f2: 1.0,
            flag2: false,
            sr2: 1.0,
            inv2: 1.0,
            sr: 1.0,
            inv_sr: 1.0,
            block: 0,
            vel: 0.0,
        }
    }

    fn bp_coefs(&mut self, fc: f32) {
        let half = self.sr1 * f(0x3efa_e148);
        let c = if fc < 0.0 { 0.0 } else { fc };
        self.f1 = if c > half { half } else { c };
        let g = tan3(self.inv1 * (fc * PI));
        self.g1 = g;
        self.gk1 = g * (g + self.k1);
        self.h1 = 1.0 / (self.gk1 + 1.0);
    }

    pub(crate) fn lp_coefs(&mut self, fc: f32) {
        self.f2 = fc;
        let g = (self.inv2 * (fc * TWO_PI)).min(f(0x3ff3_3333));
        self.g2 = g;
        let num = (g + 2.0) * (2.0 - g) * self.k2;
        let den = g.mul_add(self.k2, 2.0) - g;
        self.c2 = num / (den + den);
    }

    fn set_sample_rate(&mut self, sr: f32, block: i32) {
        if !(1.0..=500_000.0).contains(&sr) {
            return;
        }
        let inv = 1.0 / sr;
        self.sr = sr;
        self.inv_sr = inv;
        self.block = block;
        self.env.set_sample_rate(sr);
        self.sr1 = sr;
        self.inv1 = inv;
        let old = self.f1;
        self.f1 = -1.0;
        if old != -1.0 {
            self.bp_coefs(old);
        }
        if self.f1 != self.bp_hz {
            let f = self.bp_hz;
            self.bp_coefs(f);
        }
        if self.flag1 {
            self.flag1 = false;
            self.k1 = 2.0;
            self.gk1 = self.g1 * (self.g1 + 2.0);
            self.h1 = 1.0 / (self.gk1 + 1.0);
        }
        self.sr2 = sr;
        self.inv2 = inv;
        let old = self.f2;
        self.f2 = -1.0;
        if old != -1.0 {
            self.lp_coefs(old);
        }
        if self.f2 != self.lp_hz {
            let f = self.lp_hz;
            self.lp_coefs(f);
        }
        if self.flag2 {
            self.flag2 = false;
            self.k2 = 2.0;
            let f = self.f2;
            self.lp_coefs(f);
        }
        // NoiseGen<white>::setSampleRate: the scale (the pink filter is not used).
        self.scale = (sr / 44100.0).sqrt() * RND;
    }

    fn note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        self.vel = v;
        let [lo, hi, s1, s2] = self.vc;
        let t = curve2(v, lo, hi, s1, s2);
        if t > 0.0 {
            if self.reseed {
                self.seed = self.seed0;
            }
            self.env.note_on(t);
        }
    }

    #[inline]
    fn process(&mut self) -> f32 {
        self.seed = lcg(self.seed);
        let x = self.gain * (self.scale * self.seed as f32);
        let [a, b, c, ..] = self.s;
        let t = b * self.g1.mul_add(2.0, self.k1);
        let fb = c + a.mul_add(self.gk1, t);
        let hp = self.h1 * (x - fb);
        let bp = (a + hp).mul_add(self.g1, b);
        let lp = (b + bp).mul_add(self.g1, c);
        self.s = [hp, bp, lp, hp, bp, lp];
        let x = (hp + self.x1) * 0.25;
        self.x1 = hp;
        let v1 = self.g2.mul_add(x - self.c2.mul_add(self.z1, self.z2), self.z1);
        let v2 = self.g2.mul_add(v1, self.z2);
        let out = self.z2 + v2;
        let two = v1 + v1;
        self.o = [hp - two.mul_add(self.c2, out), two, out];
        self.z1 = v1;
        self.z2 = v2;
        out * self.env.tick()
    }
}

/// `NI::DSP::AttackCore<…>` (OrangeSDCore +0x100).
#[derive(Clone)]
pub(crate) struct Attack {
    pub main: MainOsc,
    pub fluffy: Fluffy,
    pub noise: Noise,
    /// +0x7e0 sr, +0x7e4 control period.
    pub sr: f32,
    pub period: i32,
    /// +0x7e8..: (dB, gain) of the click, the oscillator and the noise.
    pub db: [f32; 3],
    pub gain: [f32; 3],
    /// +0x800 click frequency, +0x804 its mode, +0x808 its length, +0x80c the delay
    /// of the body after the attack (ms).
    pub click_hz: f32,
    pub click_mode: f32,
    pub click_len: i32,
    pub delay_ms: f32,
    /// +0x810 pending velocity, +0x814 the oscillator/noise still to start.
    pub pend_vel: f32,
    pub pending: bool,
    /// +0x818..+0x824 envelope times copied for `updateDelays`.
    pub t818: f32,
    pub t81c: f32,
    pub t820: f32,
    pub t824: f32,
    /// +0x828 sounding.
    pub active: bool,
    /// +0x840 gains now, +0x850 steps, +0x860 targets (control-rate ramp).
    pub cur: [f32; 4],
    pub step: [f32; 4],
    pub target: [f32; 4],
    /// +0x870 start a ramp, +0x871 ramping, +0x874 steps left, +0x878 ramp steps,
    /// +0x87c 1/(steps·period), +0x880 ramp state.
    pub start: bool,
    pub ramping: bool,
    pub left: i32,
    pub steps: i32,
    pub inv_steps: f32,
    pub rstate: i32,
}

impl Attack {
    pub(crate) fn new() -> Attack {
        Attack {
            main: MainOsc::new(),
            fluffy: Fluffy::new(),
            noise: Noise::new(),
            sr: -1.0,
            period: -1,
            db: [0.0; 3],
            gain: [0.0; 3],
            click_hz: 0.0,
            click_mode: 0.0,
            click_len: 0,
            delay_ms: 0.0,
            pend_vel: 0.0,
            pending: false,
            t818: 0.0,
            t81c: 0.0,
            t820: 0.0,
            t824: 0.0,
            active: false,
            cur: [0.0; 4],
            step: [0.0; 4],
            target: [0.0; 4],
            start: false,
            ramping: false,
            left: 0,
            steps: 0,
            inv_steps: 0.0,
            rstate: 1,
        }
    }

    /// `AttackCore::updateDelays`: the click length and the body delay.
    pub(crate) fn update_delays(&mut self) {
        let m = if self.click_mode == 1.0 { 2.0 } else { 1.0 };
        let per = 1.0 / (self.click_hz * m);
        self.click_len = trunc_up(self.sr * per) as i32;
        let a = exp_e(self.t818.mul_add(DB, f(0x4416_8363)));
        let b = exp_e((self.t81c + self.t820).mul_add(DB, f(0x4416_8363)));
        self.delay_ms = (per + (a + b)) * 1000.0;
    }

    /// The part of `OrangeSDCore::initPreset` that sets up the attack.
    pub(crate) fn init_preset(&mut self) {
        let e = |i: u32, fr: u32| exp_e(i as f32 + f(fr));
        self.db = [-4.5, -5.0, -3.0];
        self.gain = [e(1155, 0x3e1d_a000), e(1150, 0x3e2f_2000), e(1170, 0x3dd2_4000)];
        self.target[..3].copy_from_slice(&self.gain.clone());
        self.start = true;
        self.rstate = 1;
        let fl = &mut self.fluffy;
        fl.mode = 1.0;
        fl.ceil = 0.0;
        fl.len = trunc_up(fl.sr * (1.0 / (fl.freq + fl.freq)));
        self.click_mode = 1.0;
        self.update_delays();
        let fl = &mut self.fluffy;
        fl.gain = 1.0;
        fl.freq = 337.52;
        self.click_hz = 337.52;
        fl.inc = clamp11((fl.inv_sr + fl.inv_sr) * 337.52);
        let k = if fl.mode != 1.0 { f(0x3b42_2b53) } else { f(0x3ac2_2b53) };
        fl.len = trunc_up(fl.sr * k);
        self.update_delays();
        let fl = &mut self.fluffy;
        fl.shape = 0.86;
        fl.vc = [0.76, 1.0, 0.26, 1.0];

        let n = &mut self.noise;
        n.b18 = false;
        n.reseed = false;
        n.gain_par = f(0x3e92_6e98);
        let g = clamp01(n.gain_mod + n.gain_par);
        n.gain = g * g * 4.0;
        n.env.attack_rate = n.env.inv_sr * f64::from(e(1802, 0x3f6e_1000));
        n.env.hold_len = f64::from(n.env.sr * (e(602, 0x3d58_c000) + MS));
        n.env.decay_rate = n.env.inv_sr * f64::from(e(1847, 0x3f46_a800));
        n.env_pars = [-0.5, 0.0, -5.0, 1.0];
        n.env.sustain = 1.0;
        n.release0 = 32.8;
        n.env.attack = -0.5;
        n.env.hold = 0.0;
        n.env.decay = -5.0;
        n.env.release = 32.8;
        let d = n.env.inv_sr * f64::from(e(1471, 0x3d8e_0000));
        n.env.release_mul = if d < 1.0 { 1.0 - d } else { 0.0 };
        n.bp_hz = f(0x43f6_f189);
        if n.f1 != n.bp_hz {
            n.f1 = (n.sr1 * f(0x3efa_e148)).min(n.bp_hz);
            let w = n.inv1 * f(0x44c1_f2f0);
            let g = tan3(w);
            n.g1 = g;
            n.gk1 = g * (n.k1 + g);
            n.h1 = 1.0 / (n.gk1 + 1.0);
        }
        n.lp_pitch = 93.5;
        let hz = exp_e((n.lp_mod + 93.5).mul_add(5.0, HZ0));
        let hz = if hz < 20.0 { 20.0 } else { hz };
        let hz = if hz > 20000.0 { 20000.0 } else { hz };
        n.lp_hz = hz;
        if n.f2 != hz {
            n.lp_coefs(hz);
        }
        // +0x6d4 = 1 (an int in ParallelNoise, unused here).
        n.vc = [0.0, 0.91, f(0xbed9_1687), -1.0];

        let m = &mut self.main;
        m.fscale = f(0x3f64_bc6a);
        m.inc = clamp11(m.inv_sr * (m.ratio * m.fscale * 200.0));
        m.width = MainOsc::aa_width(m.inc);
        m.freq = 200.0;
        m.skew = 0.0;
        m.restart = true;
        for o in &mut m.osc {
            o.reset_phase = true;
        }
        m.osc_attack = -24.0;
        let att = e(2037, 0x3e00_e000);
        for o in &mut m.osc {
            o.pitch = -24.0;
            o.env.attack = -24.0;
            o.env.attack_rate = o.env.inv_sr * f64::from(att);
        }
        m.osc_amp = 0.0;
        self.t818 = -24.0;
        m.copies[0] = -24.0;
        m.env_amp.attack = -24.0;
        m.env_amp.attack_rate = m.env_amp.inv_sr * f64::from(att);
        self.update_delays();
        let m = &mut self.main;
        self.t81c = f(0x41a0_a3d7);
        m.copies[1] = f(0x41a0_a3d7);
        m.env_amp.hold = f(0x41a0_a3d7);
        m.env_amp.hold_len = f64::from(m.env_amp.sr * (e(802, 0x3e29_d000) + MS));
        self.update_delays();
        let m = &mut self.main;
        self.t824 = 25.0;
        m.env_amp.release = 25.0;
        m.env_amp.release_mul = {
            let d = m.env_amp.inv_sr * f64::from(e(1548, 0x3f4d_7000));
            if d < 1.0 { 1.0 - d } else { 0.0 }
        };
        m.penv_amt = 0.64;
        m.env_pitch.attack = 1.0;
        m.env_pitch.hold = 0.0;
        m.env_pitch.attack_rate = m.env_pitch.inv_sr * f64::from(e(1787, 0x3f7b_3000));
        m.env_pitch.hold_len = f64::from(m.env_pitch.sr * (e(602, 0x3d58_c000) + MS));
        m.copies[2..6].copy_from_slice(&[25.0, 1.0, 0.0, 5.0]);
        m.env_pitch.release = 5.0;
        m.env_pitch.release_mul = {
            let d = m.env_pitch.inv_sr * f64::from(e(1748, 0x3df2_0000));
            if d < 1.0 { 1.0 - d } else { 0.0 }
        };
        m.drive_amt = 1.0;
        m.env_drive.attack = -24.0;
        m.env_drive.attack_rate = m.env_drive.inv_sr * f64::from(att);
        m.copies[6] = -24.0;
        m.copies[7] = 8.0;
        m.env_drive.hold = 8.0;
        m.env_drive.hold_len = f64::from(m.env_drive.sr * (e(681, 0x3f47_7800) + MS));
        m.copies[8] = 6.0;
        m.env_drive.release = 6.0;
        m.env_drive.release_mul = {
            let d = m.env_drive.inv_sr * f64::from(e(1738, 0x3e1c_0000));
            if d < 1.0 { 1.0 - d } else { 0.0 }
        };
        m.vc2 = [0.38, 1.0, 0.0, 1.0];
        let oscs = [
            (858.0, 22.0, 0.08f32, 1578, 0x3f33_2800, false),
            (1470.0, 27.0, f(0x3d75_c2aa), 1528, 0x3f5e_f000, false),
            (2904.0, 15.0, 0.07, 1648, 0x3eeb_b000, true),
        ];
        for (i, &(hz, rel, amp, ti, tf, b)) in oscs.iter().enumerate() {
            let o = &mut m.osc[i];
            o.on = true;
            o.amp = amp;
            o.freq = hz;
            o.release = rel;
            o.env.release = rel;
            let d = o.env.inv_sr * f64::from(e(ti, tf));
            o.env.release_mul = if d < 1.0 { 1.0 - d } else { 0.0 };
            o.b0d = b;
            o.gain = 1.0;
        }
        m.osc[2].gain = -1.0;
        m.osc[3].on = false;
        m.coupling = f(0x3e99_999a);
        m.vc1 = [f(0x3d75_c28f), 1.0, f(0xbec8_b439), f(0xbdcc_cccd)];
        m.vc_cut = [f(0x3dcc_cccd), f(0x3f59_999a), 1.0, -1.0];
        m.cut_lo = f(0x41fa_8f5c);
        m.cut_hi = 135.0;
    }

    /// The attack's part of `OrangeSDCore::onSetSampleRate`.
    pub(crate) fn set_sample_rate(&mut self, sr: f32, period: i32, steps: i32, inv_steps: f32) {
        self.sr = sr;
        self.period = period;
        self.steps = steps;
        self.inv_steps = inv_steps;
        self.main.set_sample_rate(sr);
        self.fluffy.set_sample_rate(sr);
        self.noise.set_sample_rate(sr, period);
        self.update_delays();
    }

    /// The attack's part of `OrangeSDCore::onReset`.
    pub(crate) fn reset(&mut self) {
        self.pending = false;
        self.active = false;
        self.cur = self.target;
        self.start = true;
        self.rstate = 1;
        let m = &mut self.main;
        m.phase = 1.0;
        m.inc = 0.0;
        m.seed = 1;
        for e in [&mut m.env_amp, &mut m.env_pitch, &mut m.env_drive] {
            e.state = 0;
            e.cur = 0.0;
        }
        for o in &mut m.osc {
            o.phase = 0.0;
            o.fa8 = 0.0;
        }
        m.lp = 0.0;
        m.hp = 0.0;
        m.lp2 = 0.0;
        m.hp2 = 0.0;
        m.last_cut = -1.0;
        self.fluffy.phase = -1.0;
        self.fluffy.on = false;
        self.fluffy.count = 0;
        let n = &mut self.noise;
        n.seed = n.seed0;
        n.env.state = 0;
        n.env.cur = 0.0;
        n.s = [0.0; 6];
        n.x1 = 0.0;
        n.z1 = 0.0;
        n.z2 = 0.0;
        n.o = [0.0; 3];
    }

    /// `AttackCore::setNoteOn`: the click now, the oscillator and noise when it ends.
    pub(crate) fn note_on(&mut self, v: f32) {
        if !(0.0..=1.0).contains(&v) {
            return;
        }
        if !self.fluffy.on && self.main.env_amp.state == 0 && self.noise.env.state == 0 && !self.active {
            self.cur = self.target;
            self.start = true;
            self.rstate = 1;
        }
        self.active = true;
        if self.pending {
            self.main.note_on(v);
            self.noise.note_on(v);
            self.pending = false;
        } else {
            self.pend_vel = v;
            self.fluffy.note_on(v);
            self.pending = true;
        }
    }

    /// `AttackCore::setModulationFreqPhOscRatio(semitones)`: the tune of the oscillator.
    pub(crate) fn set_ratio(&mut self, st: f32) {
        if !(-64.0..=64.0).contains(&st) {
            return;
        }
        let r = exp_e(st.mul_add(5.0, 1200.0)).max(0.0);
        let m = &mut self.main;
        m.ratio = r;
        m.inc = clamp11(m.inv_sr * (m.freq * (r * m.fscale)));
        m.width = MainOsc::aa_width(m.inc);
        for o in &mut m.osc {
            o.ratio = r;
        }
    }

    /// `AttackCore::setModulationPNoiseEnvDec2(x)`: the noise release.
    pub(crate) fn set_noise_release(&mut self, x: f32) {
        if !(-28.0..=40.0).contains(&x) {
            return;
        }
        let n = &mut self.noise;
        n.rel_mod = x;
        let t = n.release0 + x;
        n.env.release = t;
        n.env.release_mul = release_mul(n.env.inv_sr, t);
    }

    /// `AttackCore::setModulationNoiseLP(x)`: the noise low-pass (semitones).
    pub(crate) fn set_noise_lp(&mut self, x: f32) {
        if !(-72.0..=60.0).contains(&x) {
            return;
        }
        let n = &mut self.noise;
        n.lp_mod = x;
        let hz = exp_e((n.lp_pitch + x).mul_add(5.0, HZ0));
        let hz = if hz < 20.0 { 20.0 } else { hz };
        let hz = if hz > 20000.0 { 20000.0 } else { hz };
        n.lp_hz = hz;
        if n.f2 != hz {
            n.lp_coefs(hz);
        }
    }

    /// The attack's gain ramp (inlined in `OrangeSDCore::processControlRate`): a
    /// change of target starts a linear ramp over `steps` control periods.
    pub(crate) fn control_rate(&mut self) {
        if self.start {
            self.start = false;
            self.rstate = 2;
            let mut any = false;
            for i in 0..4 {
                let st = (self.target[i] - self.cur[i]) * self.inv_steps;
                self.step[i] = st;
                if st * st < 1e-15 {
                    self.step[i] = 0.0;
                    self.cur[i] = self.target[i];
                } else {
                    any = true;
                }
            }
            if any {
                self.left = self.steps;
                self.ramping = true;
            } else {
                self.rstate = 4;
            }
            return;
        }
        self.left = self.left.wrapping_sub(1);
        if self.left == 0 {
            self.cur = self.target;
            self.step = [0.0; 4];
            self.ramping = false;
            self.rstate = 4;
            return;
        }
        if self.rstate == 2 {
            self.rstate = 3;
        } else if self.rstate == 4 {
            self.rstate = 0;
        }
    }

    #[inline]
    pub(crate) fn process(&mut self) -> f32 {
        for i in 0..4 {
            self.cur[i] += self.step[i];
        }
        let click = if self.fluffy.on {
            self.fluffy.process()
        } else {
            if self.pending {
                let v = self.pend_vel;
                self.note_on(v);
            }
            0.0
        };
        let main = self.main.process();
        let noise = self.noise.process();
        let (g0, g1, g2) = if self.ramping {
            (self.cur[0], self.cur[1], self.cur[2])
        } else {
            (self.gain[0], self.gain[1], self.gain[2])
        };
        if !self.fluffy.on && self.main.env_amp.state == 0 && self.noise.env.state == 0 {
            self.active = false;
        }
        g2.mul_add(noise, g0.mul_add(click, main * g1))
    }
}
