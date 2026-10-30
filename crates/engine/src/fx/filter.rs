//! Maschine's Filter (docs/fx-filter.md): a saturating state-variable filter
//! (`FXCORE::FilterCore`, §5) whose cutoff an LFO (`LEGACY::SyncableLFOHelper`, §4) or an
//! envelope follower (§5.3) modulates. The follower can listen to the side-chain input,
//! which the side-chain filter (docs/fx-compressor.md §4) shapes first (in Mode BP only,
//! §3).

use super::sidechain::SidechainFilter;
use super::{Effect, param};
use std::sync::OnceLock;

/// An `f32` constant as Maschine stores it.
const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}
const D: f32 = c(0x2193_92ef); // 1e-18, the core's denormal offset
const LN2: f32 = c(0x3f31_7215); // 0.693147
const PI: f32 = c(0x4049_0fd0); // 3.14159 (not quite π)
const TINY: f32 = c(0x2edb_e6ff); // 1e-10

/// The LFO rates as `(numerator, denominator)` bars, slowest first (§4).
const RATES: [(i32, i32); 12] = [(16, 1), (8, 1), (4, 1), (2, 1), (1, 1), (1, 2), (3, 8), (1, 4), (3, 16), (1, 8), (1, 16), (1, 32)];

/// Per Mode (LP, BP, HP, Notch): the resonance scale, the resonance above which the
/// output gain drops, and by how much at full resonance (§5.2).
const RES_SCALE: [f32; 4] = [c(0x3f7e_f9db), c(0x3f7e_f9db), c(0x3f7e_f9db), 0.0];
const GAIN_KNEE: [f32; 4] = [0.5, 0.0, 0.5, 0.0];
const GAIN_DROP: [f32; 4] = [c(0x3ed7_0a3d), c(0x3f2e_147b), c(0x3ed7_0a3d), 0.0];

/// The follower's gate-release time and gate threshold at Shape −1, 0, +1 (§5.3).
const ENV_TIME: [f32; 3] = [c(0x3851_b717), c(0x38d1_b717), 0.25];
const ENV_THRESHOLD: [f32; 3] = [c(0x3f6e_147b), 0.125, c(0x3f4c_cccd)];

/// `(int)(x ± 0.5)` in `f64`, the sign by `x < 0`: Maschine's round-half-away.
fn round(x: f32) -> i32 {
    (x as f64 + if x < 0.0 { -0.5 } else { 0.5 }) as i32
}

/// `2^n` for an integer `n`, built from the exponent bits.
fn pow2i(n: i32) -> f32 {
    f32::from_bits(0x3f80_0000i32.wrapping_add(n.wrapping_shl(23)) as u32)
}

/// `FilterCore`'s table: `2^(i/1021 − 0.5)` for `i` in 0..=1021 as 6-digit decimals,
/// the last entry repeated.
fn exp2_table() -> &'static [f32; 1023] {
    static T: OnceLock<[f32; 1023]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0f32; 1023];
        for (i, v) in t.iter_mut().enumerate().take(1022) {
            let x = (i as f64 / 1021.0 - 0.5).exp2() as f32;
            *v = format!("{:.5e}", x as f64).parse::<f64>().unwrap() as f32;
        }
        t[1022] = t[1021];
        t
    })
}

/// `2^x` through that table (only `endInit` uses it, for the smoothing coefficient).
fn exp2_lookup(x: f32) -> f32 {
    let n = round(x);
    let f = ((x - n as f32) + 0.5) * 1021.0;
    let i = round(f - 0.5);
    let k = (i as u32 & 0x3ff) as usize;
    let t = f - i as f32;
    let tab = exp2_table();
    ((1.0 - t) * tab[k] + t * tab[k + 1]) * pow2i(n)
}

/// `exp10` of Apple's libm.
fn exp10(x: f64) -> f64 {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn __exp10(x: f64) -> f64;
        }
        // SAFETY: a pure libm function.
        unsafe { __exp10(x) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        10f64.powf(x)
    }
}

/// `NI::MASCOMMON::LEGACY::SyncableLFOHelper` over [`RATES`] (§4).
struct Lfo {
    /// 0 off (Env), 1 free, 2 synced.
    mode: i32,
    speed: f32,
    /// The free rate (Hz), and the synced cycle (beats).
    free: f32,
    beats: f32,
    /// The rate for this block (Hz).
    out: f32,
    /// Phase (cycles) added to the song position when synced.
    offset: f32,
    /// The phase the LFO restarts at this block, synced with the transport running.
    reset: Option<f32>,
}

impl Lfo {
    fn new() -> Lfo {
        let mut l = Lfo { mode: 1, speed: 0.5, free: 0.0, beats: 0.0, out: 0.0, offset: 0.0, reset: None };
        l.free = l.free_rate();
        l.beats = l.synced_beats();
        l.out = l.free;
        l
    }

    fn ratio(i: usize) -> f32 {
        RATES[i].0 as f32 / RATES[i].1 as f32
    }

    /// Between the two nearest rates: `1 / (2·bars)` Hz.
    fn free_rate(&self) -> f32 {
        let pos = 11.0 * self.speed;
        let i = round(pos - 0.5).clamp(0, 10);
        let f = pos - i as f32;
        let s = (1.0 - f).mul_add(Lfo::ratio(i as usize), f * Lfo::ratio(i as usize + 1));
        1.0 / (s + s)
    }

    fn synced_beats(&self) -> f32 {
        let i = round(11.0 * self.speed).clamp(0, 11);
        4.0 * Lfo::ratio(i as usize)
    }

    /// `updateSpeed`.
    fn update(&mut self) {
        match self.mode {
            1 => self.free = self.free_rate(),
            2 => self.beats = self.synced_beats(),
            _ => {}
        }
    }

    /// `newFrame`: the rate, and with the transport at `quarters` (playing), synced, the
    /// phase from the song position.
    fn frame(&mut self, bpm: f32, quarters: Option<f64>) {
        self.reset = None;
        match self.mode {
            1 => self.out = self.free,
            2 => {
                if let Some(q) = quarters {
                    let x = super::song_position(q) / self.beats as f64 + self.offset as f64;
                    let n = (x + if x < 0.0 { -0.5 } else { 0.5 }) as i32;
                    self.reset = Some((x - n as f64) as f32);
                }
                self.out = bpm * c(0x3c88_8889) / self.beats;
            }
            _ => {}
        }
    }
}

/// `FXCORE::FilterCore`: inputs, smoothed controls, modulators and the two filters.
#[derive(Default)]
struct Core {
    sr: f32,
    /// Control ticks come every `every + 1` samples; samples since the last.
    every: i32,
    counter: i32,
    /// The control rate, and π/sr.
    rate: f32,
    pi_sr: f32,
    // Inputs as the processor last set them (§2).
    mode_in: f32,
    res_in: f32,
    cut_in: f32,
    amount_in: f32,
    env_in: f32,
    lfo_shape_in: f32,
    lfo_hz: f32,
    phase_in: f32,
    env_shape_in: f32,
    decay_in: f32,
    smooth_in: f32,
    sc: bool,
    /// Bits: cutoff (1), resonance (2), amount (4) still smoothing; any → all do.
    flags: u32,
    /// The smoothers (in `f64`): coefficient, targets, current values.
    smooth: f64,
    res_t: f64,
    res_c: f64,
    cut_t: f64,
    cut_c: f64,
    amt_t: f64,
    amt_c: f64,
    res: f32,
    cut: f32,
    amt: f32,
    /// The cutoff pitch and Amount as set.
    cut_pitch: f32,
    amount: f32,
    /// A full modulation's pitch span: down (`cut − min`) and up (`max − cut`).
    span: [f32; 2],
    span_max: f32,
    span_min: f32,
    res_scale: f32,
    damp: f32,
    gain_knee: f32,
    gain_inv: f32,
    gain_drop: f32,
    last_res: f32,
    gain: f32,
    out_idx: usize,
    // LFO (§5.3).
    env_mode: bool,
    shape_n: u32,
    inc: f32,
    phase: f32,
    l134: f32,
    l138: f32,
    l13c: f32,
    l140: f32,
    l144: f32,
    l148: f32,
    lfo: f32,
    lfo_slew: f32,
    // Envelope follower (§5.3).
    env_out: f32,
    env_shape: f32,
    threshold: f32,
    hold: i32,
    hold_cur: i32,
    count: i32,
    env: f32,
    attack: f32,
    release_gate: f32,
    release: f32,
    smooth_coef: f32,
    env_time: f32,
    gate_decay: f32,
    // The filter's coefficients and saturation (§5.1).
    g: f32,
    k1: f32,
    k2: f32,
    k3: f32,
    lim: f32,
    q: f32,
    /// States per channel (L, R): band-pass, high-pass, low-pass.
    z1: [f32; 2],
    zh: [f32; 2],
    z2: [f32; 2],
}

impl Core {
    // ---- Inputs (`processFloatParameter` / `processEnumParameter`, §2) ----

    /// Mode: the output, its resonance scale and gain drop.
    fn set_mode(&mut self, mode: f32) {
        self.mode_in = mode;
        let m = (mode as f64 + 0.5) as i32;
        let k = (round(m as f32) as u32).min(3) as usize;
        self.out_idx = (round(round(m as f32) as f32) as u32).min(3) as usize;
        self.gain_drop = GAIN_DROP[k];
        self.res_scale = RES_SCALE[k];
        let d = 1.0 - self.res_scale * self.res;
        self.damp = d + d;
        let s = self.g + self.damp;
        self.k1 = self.g + s;
        self.k2 = self.g * s;
        self.k3 = 1.0 / (self.k2 + 1.0);
        self.gain_knee = GAIN_KNEE[k].max(c(0x3a83_126f));
        self.gain_inv = 1.0 / (1.0 - self.gain_knee);
        self.gain = 1.0 - self.gain_drop * (self.gain_inv * (self.last_res - self.gain_knee));
    }

    /// Cutoff, as the core's normalized pitch `(note − 30)/97`.
    fn set_cutoff(&mut self, p: f32) {
        self.cut_in = p;
        self.cut_pitch = p * (2.0 - p) * 139.0 + -12.0;
        self.span = [self.cut_pitch - self.span_min, self.span_max - self.cut_pitch];
        self.cut_t = self.cut_pitch as f64;
        self.amt_t = (self.span[(self.amount > 0.0) as usize] * self.amount) as f64;
        self.flags |= 5;
    }

    fn set_resonance(&mut self, v: f32) {
        self.res_in = v;
        self.res_t = ((2.0 - v) * v) as f64;
        self.flags |= 2;
    }

    fn set_amount(&mut self, v: f32) {
        self.amount_in = v;
        self.amount = v;
        self.amt_t = (self.span[(v > 0.0) as usize] * v) as f64;
        self.flags |= 4;
    }

    /// Source: LFO (false) or Env; the modulation span follows.
    fn set_source(&mut self, env: bool) {
        self.env_in = env as u8 as f32;
        let (lo, hi) = if env { (24.0, 240.0) } else { (-12.0, 127.0) };
        self.env_mode = env;
        self.span_max = hi;
        self.span_min = lo;
        self.span = [self.cut_pitch - lo, hi - self.cut_pitch];
        self.amt_t = (self.span[(self.amount > 0.0) as usize] * self.amount) as f64;
        self.flags |= 4;
    }

    fn set_lfo_shape(&mut self, v: f32) {
        self.lfo_shape_in = v;
        let a = 1.0 - (v + -0.75) * 4.0;
        let b = 1.0 - (v + v);
        self.l140 = (v + -0.5) * 4.0;
        self.l144 = if a > TINY { a } else { TINY };
        self.l148 = 1.0 / self.l144;
        let n = ((v > 0.5) as u8 as f32 + (v > 0.75) as u8 as f32) as f64 + 0.5;
        self.shape_n = (n as i32 as u32) & 3;
        self.l134 = if b > TINY { b * 0.5 } else { c(0x2e5b_e6ff) };
        self.l138 = 1.0 / self.l134;
        self.l13c = 1.0 / (1.0 - self.l134);
    }

    /// Decay: the follower's hold and release.
    fn set_decay(&mut self, v: f32) {
        self.decay_in = v;
        let t = if self.env_shape > 0.4 { self.env_time } else { c(0x38d1_b717) };
        let s = ((v.abs() * 0.5 + 0.5) * v * c(0x3e42_8f5c) + c(0x3c23_d70a)) * self.rate;
        let r = self.rate * t;
        self.hold = round(s);
        self.gate_decay = if r < 1.0 { c(0x3e9d_1bd6) } else { 1.0 - LN2 / r };
        self.release = if s < 1.0 { LN2 } else { LN2 / s };
    }

    /// Smooth: the follower's output smoothing (0.6..70 ms).
    fn set_smooth(&mut self, v: f32) {
        self.smooth_in = v;
        let t = (v * v * c(0x428a_cccd) + c(0x3f19_999a)) * (self.rate / 1000.0);
        self.smooth_coef = if t < 1.0 { LN2 } else { LN2 / t };
    }

    /// Shape: gate (≤ 0) or follower, the gate's threshold and release.
    fn set_env_shape(&mut self, v: f32) {
        self.env_shape_in = v;
        let u = (v + 1.0) * 0.5;
        self.env_shape = u;
        let x = u + u;
        let i = round(x + -0.5);
        let k0 = (i as u32).min(2) as usize;
        let k1 = (round(i as f32 + 1.0) as u32).min(2) as usize;
        let f = x - i as f32;
        self.env_time = ENV_TIME[k0] + (ENV_TIME[k1] - ENV_TIME[k0]) * f;
        self.threshold = ENV_THRESHOLD[k0] + (ENV_THRESHOLD[k1] - ENV_THRESHOLD[k0]) * f;
        let r = (if u > 0.4 { self.env_time } else { c(0x38d1_b717) }) * self.rate;
        self.gate_decay = if r < 1.0 { c(0x3e9d_1bd6) } else { -LN2 / r + 1.0 };
    }

    /// The LFO's rate (Hz) for the block (`FilterProcessor::process`).
    fn set_lfo_rate(&mut self, hz: f32) {
        self.lfo_hz = hz;
        self.lfo_slew = hz * 48.0 * c(0x38da_f153) * (44100.0 / self.rate);
        self.inc = hz * (1.0 / self.rate);
    }

    /// `resume`: everything clears but the inputs, `endInit` derives the rest from
    /// them, and runs one control tick and sample on silence.
    fn resume(&mut self) {
        let old = std::mem::take(self);
        self.sr = old.sr;
        self.pi_sr = PI / self.sr;
        let factor = round(self.sr / 11025.0);
        self.rate = if factor != 0 { (round(self.sr) / factor) as f32 } else { 0.0 };
        self.every = factor - 1;
        // 1 − 2^((… − 61) / max(rate/10 − 1, 1)).
        let num = c(0x3e9e_defc) * c(0x3f75_d200) + c(0x3e9f_7447) * c(0x3d22_e000) + -61.0;
        let den = ((self.rate as f64 / 1000.0) * 100.0 + -1.0).max(1.0);
        self.smooth = 1.0 - exp2_lookup((num as f64 / den) as f32) as f64;
        self.attack = LN2; // its time is 0 (§5.3)
        self.set_source(old.env_in > 0.5);
        self.set_cutoff(old.cut_in);
        self.set_resonance(old.res_in);
        self.set_amount(old.amount_in);
        self.set_lfo_shape(old.lfo_shape_in);
        self.set_env_shape(old.env_shape_in);
        self.set_decay(old.decay_in);
        self.set_smooth(old.smooth_in);
        self.set_lfo_rate(old.lfo_hz);
        self.phase_in = old.phase_in;
        self.phase = old.phase_in;
        // The smoothers start at their targets.
        (self.res_c, self.cut_c, self.amt_c) = (self.res_t, self.cut_t, self.amt_t);
        self.res = self.res_c as f32;
        self.cut = self.cut_c as f32;
        self.amt = self.amt_c as f32;
        self.set_mode(old.mode_in);
        self.gain = 1.0;
        self.flags = 7;
        self.step(0.0, 0.0, (0.0, 0.0), true);
        self.counter = 0;
    }

    /// One control tick: modulators, smoothing, the filter's coefficients (§5.2).
    fn control(&mut self, xl: f32, xr: f32, sc: (f32, f32)) {
        let mut m = 0.0;
        if !self.env_mode {
            let next = self.phase + self.inc;
            let p = self.phase + 0.5;
            let q = 1.0 - p;
            let v = match self.shape_n {
                // Saw ↔ triangle.
                0 => 1.0 - if q.abs() >= self.l134 { p * self.l13c } else { q * self.l138 },
                // → parabolic sine.
                1 => {
                    let u = (0.5 - p) + (0.5 - p);
                    q + ((u * (2.0 - u.abs()) + 1.0) * 0.5 - q) * self.l140
                }
                // → square.
                _ => {
                    let a = ((0.5 - p) + (0.5 - p)).abs();
                    let s = a * self.l148;
                    let w = 2.0 - s.abs();
                    let sign = if p > 0.5 { -1.0 } else { 1.0 };
                    let y = if a < self.l144 { sign * s * w } else { sign };
                    (y + 1.0) * 0.5
                }
            };
            self.lfo += self.lfo_slew * ((v - self.lfo) + D);
            m = self.lfo;
            self.phase = next - round(next) as f32;
        }
        if self.flags as i32 >= 1 {
            self.res_c += self.smooth * ((self.res_t - self.res_c) + 1e-100);
            self.res = self.res_c as f32;
            self.amt_c += self.smooth * ((self.amt_t - self.amt_c) + 1e-100);
            self.amt = self.amt_c as f32;
            self.cut_c += self.smooth * ((self.cut_t - self.cut_c) + 1e-100);
            self.cut = self.cut_c as f32;
            let d = 1.0 - self.res_scale * self.res;
            self.damp = d + d;
            let mut f = 0.0;
            if self.gain_knee < self.res {
                self.last_res = self.res;
                f = (self.res - self.gain_knee) * self.gain_inv * self.gain_drop;
            }
            self.gain = 1.0 - f;
        }
        if self.env_mode {
            let (a, b) = if self.sc { (sc.0.abs(), sc.1) } else { (xl.abs(), xr) };
            let b = b.abs();
            let level = if a > b { a } else { b };
            let target = if self.env_shape > 0.4 {
                level
            } else {
                // Gate: open while above the threshold, and `hold` ticks after.
                let (n, h) = if level > self.threshold {
                    self.hold_cur = self.hold;
                    (0, self.hold)
                } else {
                    (self.count, self.hold_cur)
                };
                self.count = n.wrapping_add(1);
                if h > n { 1.0 } else { 0.0 }
            };
            let d = target - self.env;
            let k = if d > 0.0 {
                self.release_gate = 1.0;
                self.attack
            } else {
                let old = self.release_gate;
                self.release_gate = old * self.gate_decay;
                if old > 0.5 { 0.0 } else { self.release }
            };
            let e = self.env + d * k;
            self.env = e + D;
            let x = if e < 0.0 { 0.0 } else { e };
            let x = if e < 1.0 { x } else { 1.0 };
            let a = x * (2.0 - x);
            let y = a + self.decay_in * (x - a);
            let out = self.env_out + self.smooth_coef * (y - self.env_out);
            self.env_out = out + D;
            m = out;
        }
        // The cutoff pitch → Hz (a cubic 2^x) → the prewarped gain (a tan series).
        let pitch = self.cut + m * self.amt;
        let x = if pitch > 132.0 { c(0x412f_fdc4) } else { pitch * c(0x3daa_a880) };
        let n = round(x);
        let f = x - n as f32;
        let p = f * (f * (f * c(0x3d65_6042) + c(0x3e77_ced9)) + c(0x3f31_6873)) + 1.0;
        let hz = p * pow2i(n) * c(0x4102_d6b6);
        let t = (hz + -100.0) * c(0x3904_bb19);
        let w = (self.pi_sr * hz).min(2.0);
        let w2 = w * w;
        let w3 = w * w2;
        self.g = w + (w3 * c(0x3eaa_aa9f) + w2 * w3 * c(0x3e08_8872));
        let s = self.damp + self.g;
        self.k1 = self.g + s;
        self.k2 = self.g * s;
        self.k3 = 1.0 / (self.k2 + 1.0);
        // The integrators saturate harder as the cutoff rises (24 → 0.6).
        let l = if t < 0.0 { 24.0 } else { t * c(0xc1bb_3333) + 24.0 };
        let l = if t < 1.0 { l } else { c(0x3f19_99a0) };
        let l = if l < c(0x2c0c_bccc) { c(0x2c0c_bccc) } else { l };
        self.q = 0.25 / l;
        self.lim = l + l;
    }

    /// An integrator's state, soft-clipped.
    fn clip(&self, v: f32) -> f32 {
        let v = if v > self.lim {
            self.lim
        } else if -self.lim > v {
            -self.lim
        } else {
            v
        };
        v * (1.0 - v.abs() * self.q) + D
    }

    /// `processAudio`: one sample (§5.1), after a control tick if `tick`.
    fn step(&mut self, l: f32, r: f32, sc: (f32, f32), tick: bool) -> (f32, f32) {
        let clamp = |x: f32| {
            let y = if x < -100.0 { -100.0 } else { x };
            if x > 100.0 { 100.0 } else { y }
        };
        let (l, r) = (clamp(l), clamp(r));
        if tick {
            self.control(l, r, sc);
        }
        let mut out = [0f32; 2];
        for (ch, x) in [(1usize, r), (0, l)] {
            let (z1, zh, z2) = (self.z1[ch], self.zh[ch], self.z2[ch]);
            let hp = self.k3 * (x - (z2 + (z1 * self.k1 + zh * self.k2)));
            let bp = z1 + self.g * (zh + hp);
            let two = bp + bp;
            let lp = z2 + self.g * (z1 + bp);
            self.zh[ch] = hp + D;
            self.z1[ch] = self.clip(bp);
            self.z2[ch] = self.clip(lp);
            out[ch] = [lp, two, hp, x - two][self.out_idx] * self.gain;
        }
        (out[0], out[1])
    }
}

/// Field names and defaults of the float parameters, in `tFloatParameters` order:
/// Cutoff (1 kHz), Resonance, Amount, Speed, LFO Shape, Phase, Decay, Smooth, Shape.
const PARAMS: [(&str, f32); 9] = [
    ("cutoff", 0.549_053_7),
    ("resonance", 0.5),
    ("amount", 0.0),
    ("speed", 0.5),
    ("lfo_shape", 0.5),
    ("phase", 0.0),
    ("decay", 0.5),
    ("smooth", 0.5),
    ("env_shape", 0.0),
];

/// Maschine's processing block (samples): the LFO re-phase, rate and smoothing stop come
/// once per block (§3).
const BLOCK: usize = 128;

pub struct Filter {
    core: Core,
    lfo: Lfo,
    bpm: f32,
    /// The song position (quarters) at the next block, while the transport plays.
    quarters: Option<f64>,
    /// Mode, Source, then [`PARAMS`].
    values: Option<[f32; 11]>,
    scf: SidechainFilter,
    /// The side-chain block.
    sc: [Vec<f32>; 2],
}

impl Filter {
    pub fn new(sr: f64) -> Filter {
        // The processor's core starts at 44.1 kHz.
        let mut core = Core { sr: 44100.0, ..Core::default() };
        core.resume();
        core.sr = sr as f32;
        Filter {
            core,
            lfo: Lfo::new(),
            bpm: 120.0,
            quarters: None,
            values: None,
            scf: SidechainFilter::new(sr),
            sc: [Vec::new(), Vec::new()],
        }
    }

    /// `processEnumParameter` (`k` 0 Mode, 1 Source) and `processFloatParameter` (the rest).
    fn apply(&mut self, k: usize, v: f32) {
        let core = &mut self.core;
        match k {
            0 => core.set_mode(v),
            1 => {
                let s = v as i64;
                self.lfo.mode = match s {
                    0 => 1,
                    1 => 2,
                    _ => 0,
                };
                self.lfo.update();
                core.set_source(s == 2);
            }
            2 => {
                // Cutoff: 46 Hz..12.54 kHz, as the core's normalized pitch.
                let hz = exp10((v as f64).mul_add(2.435_539_704_813_123, 1.662_757_831_681_574)).clamp(46.0, 12540.0);
                let note = (hz as f32).ln().mul_add(c(0x418a_7fac), c(0xc211_815a));
                core.set_cutoff((note + -30.0) * c(0x3c28_e83f));
            }
            3 => core.set_resonance(v),
            4 => core.set_amount(v),
            5 => {
                self.lfo.speed = v;
                self.lfo.update();
            }
            6 => core.set_lfo_shape(v),
            7 => self.lfo.offset = (v + 1.0) * 0.5, // used synced, with the transport running (§4)
            8 => core.set_decay(v),
            9 => core.set_smooth(v),
            _ => core.set_env_shape(v),
        }
    }

    /// `FilterProcessor::process` for one of Maschine's blocks: the LFO's rate (and, synced
    /// with the transport playing, phase) for the block, then the core per sample. A
    /// side-chain feeds the envelope follower.
    fn run(&mut self, l: &mut [f32], r: &mut [f32], sc: Option<(&[f32], &[f32])>) {
        let n = l.len().min(r.len());
        self.lfo.frame(self.bpm, self.quarters);
        let core = &mut self.core;
        core.set_lfo_rate(self.lfo.out);
        if let Some(p) = self.lfo.reset {
            core.phase_in = p;
            core.phase = p;
        }
        // Smoothing stops once all three are within 1e-4 of their targets.
        if (core.cut as f64 - core.cut_t).abs() <= 1e-4 {
            core.flags &= !1;
        }
        if (core.res as f64 - core.res_t).abs() <= 1e-4 {
            core.flags &= !2;
        }
        if (core.amt as f64 - core.amt_t).abs() <= 1e-4 {
            core.flags &= !4;
        }
        core.sc = sc.is_some();
        let [sl, sr] = &mut self.sc;
        let sc = sc.map(|(a, b)| {
            sl.clear();
            sl.extend_from_slice(&a[..n]);
            sr.clear();
            sr.extend_from_slice(&b[..n]);
            // Only in Mode BP (sic; §3).
            if core.mode_in == 1.0 {
                self.scf.process(sl, sr);
            }
            (&sl[..], &sr[..])
        });
        for (i, (xl, xr)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
            let tick = if core.counter < core.every {
                core.counter += 1;
                false
            } else {
                core.counter = 0;
                true
            };
            let s = sc.map_or((0.0, 0.0), |(a, b)| (a[i], b[i]));
            (*xl, *xr) = core.step(*xl, *xr, s, tick);
        }
    }
}

impl Effect for Filter {
    /// The transport for the next block: a synced LFO follows it (§4).
    fn set_transport(&mut self, quarters: Option<f64>) {
        self.quarters = quarters;
    }

    fn set(&mut self, params: &[(String, f64)]) {
        let mut new = [0f32; 11];
        new[0] = param(params, "mode", 0.0) as f32;
        new[1] = param(params, "source", 0.0) as f32;
        for (k, (name, def)) in PARAMS.iter().enumerate() {
            new[k + 2] = param(params, name, *def as f64) as f32;
        }
        match self.values {
            None => {
                // The parameters reach the processor, then it resumes at the host's rate.
                for (k, v) in new.iter().enumerate() {
                    self.apply(k, *v);
                }
                self.core.resume();
            }
            Some(old) => {
                for k in 0..11 {
                    if new[k].to_bits() != old[k].to_bits() {
                        self.apply(k, new[k]);
                    }
                }
            }
        }
        self.values = Some(new);
        self.scf.set(params);
    }

    fn set_tempo(&mut self, bpm: f64) {
        self.bpm = bpm as f32;
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.process_sc(l, r, None)
    }

    /// Maschine runs its effects in blocks of [`BLOCK`] samples (§3).
    fn process_sc(&mut self, l: &mut [f32], r: &mut [f32], sc: Option<(&[f32], &[f32])>) {
        let n = l.len().min(r.len());
        let mut at = 0;
        while at < n {
            let k = (n - at).min(BLOCK);
            let s = sc.map(|(a, b)| (&a[at..at + k], &b[at..at + k]));
            self.run(&mut l[at..at + k], &mut r[at..at + k], s);
            at += k;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(kv: &[(&str, f64)]) -> Vec<(String, f64)> {
        kv.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn sine(hz: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (i as f32 * hz / 48_000.0 * std::f32::consts::TAU).sin() * 0.25).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        let t = &x[x.len() / 2..];
        (t.iter().map(|v| v * v).sum::<f32>() / t.len() as f32).sqrt()
    }

    fn run(p: &[(String, f64)], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut f = Filter::new(48_000.0);
        f.set(p);
        let (mut l, mut r) = (x.to_vec(), x.to_vec());
        for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            f.process(a, b);
        }
        (l, r)
    }

    #[test]
    fn exp2_table_matches_maschines() {
        let t = exp2_table();
        assert_eq!(t[0].to_bits(), 0x3f35_04f7);
        assert_eq!(t[1].to_bits(), 0x3f35_246c);
        assert_eq!(t[510].to_bits(), 0x3f7f_e9c9);
        assert_eq!(t[1021].to_bits(), 0x3fb5_04d5);
        assert_eq!(t[1022].to_bits(), 0x3fb5_04d5);
    }

    #[test]
    fn low_pass_passes_lows_and_cuts_highs() {
        let p = params(&[("mode", 0.0), ("cutoff", 0.3), ("resonance", 0.0)]);
        let lo = rms(&run(&p, &sine(50.0, 24_000)).0);
        let hi = rms(&run(&p, &sine(8_000.0, 24_000)).0);
        assert!(lo > 0.15 && hi < 0.01, "{lo} {hi}");
    }

    #[test]
    fn high_pass_cuts_lows() {
        let p = params(&[("mode", 2.0), ("cutoff", 0.7), ("resonance", 0.0)]);
        let lo = rms(&run(&p, &sine(50.0, 24_000)).0);
        let hi = rms(&run(&p, &sine(10_000.0, 24_000)).0);
        assert!(lo < 0.01 && hi > 0.1, "{lo} {hi}");
    }

    #[test]
    fn notch_removes_its_center() {
        // Cutoff 0.5: the core's pitch 0.5 → 92.25 → ≈ 1.9 kHz (§5.2).
        let p = params(&[("mode", 3.0), ("cutoff", 0.4)]);
        let mut f = Filter::new(48_000.0);
        f.set(&p);
        let hz = 8.17742 * (f.core.cut_pitch / 12.0).exp2();
        let mid = rms(&run(&p, &sine(hz, 48_000)).0);
        let far = rms(&run(&p, &sine(hz * 8.0, 48_000)).0);
        assert!(mid < 0.2 * far, "{hz} {mid} {far}");
    }

    #[test]
    fn stereo_channels_are_independent() {
        let mut f = Filter::new(48_000.0);
        f.set(&params(&[("mode", 1.0)]));
        let mut l = sine(1000.0, 4800);
        let mut r = vec![0.0; 4800];
        f.process(&mut l, &mut r);
        assert!(r.iter().all(|v| v.abs() < 1e-12));
        assert!(rms(&l) > 0.01);
    }

    #[test]
    fn loud_resonant_input_stays_finite() {
        let p = params(&[("mode", 0.0), ("cutoff", 0.9), ("resonance", 1.0), ("amount", 1.0), ("speed", 1.0)]);
        let x: Vec<f32> = (0..48_000).map(|i| if i % 100 < 50 { 50.0 } else { -50.0 }).collect();
        let (l, _) = run(&p, &x);
        assert!(l.iter().all(|v| v.is_finite() && v.abs() < 1e4));
        // Silence stays clear of denormals.
        let (l, _) = run(&p, &vec![0.0; 48_000]);
        assert!(l.iter().all(|v| *v == 0.0 || v.abs() > 1e-30));
    }

    #[test]
    fn envelope_opens_the_filter() {
        // Env source, full amount: a loud input lifts the cutoff above a quiet one's.
        let p = params(&[("mode", 0.0), ("cutoff", 0.2), ("resonance", 0.0), ("source", 2.0), ("amount", 1.0), ("env_shape", 1.0)]);
        let quiet: Vec<f32> = sine(5000.0, 24_000).iter().map(|v| v * 0.01).collect();
        let gq = rms(&run(&p, &quiet).0) / 0.01;
        let gl = rms(&run(&p, &sine(5000.0, 24_000)).0);
        assert!(gl > 2.0 * gq, "{gq} {gl}");
    }

    #[test]
    fn parameter_changes_smooth() {
        let mut f = Filter::new(48_000.0);
        f.set(&params(&[("cutoff", 0.2)]));
        f.set(&params(&[("cutoff", 0.9)]));
        // About 3 ms per step of 1/e at 48 kHz.
        let (mut l, mut r) = (vec![0.0f32; 64], vec![0.0f32; 64]);
        f.process(&mut l, &mut r);
        let (c, t) = (f.core.cut, f.core.cut_t as f32);
        assert!(c < t - 1.0, "{c} {t}");
        for _ in 0..200 {
            f.process(&mut l, &mut r);
        }
        assert!((f.core.cut - t).abs() < 1e-2);
    }

    /// Synced and playing, every 128-sample block re-phases from the host block's position
    /// (held, not advanced), rounded to Maschine's 240000 ticks per quarter (§4).
    #[test]
    fn synced_position_holds_per_host_block() {
        let p = params(&[("source", 1.0), ("speed", 0.9), ("amount", 0.8)]);
        let x: Vec<f32> = (0..1024).map(|i| ((i * 7919 + 13) % 1000) as f32 / 1000.0 - 0.5).collect();
        let mut a = Filter::new(44_100.0);
        a.set(&p);
        a.set_transport(Some(0.046_439_909));
        let (mut al, mut ar) = (x.clone(), x.clone());
        a.process(&mut al, &mut ar);
        let mut b = Filter::new(44_100.0);
        b.set(&p);
        let (mut bl, mut br) = (x.clone(), x.clone());
        for (l, r) in bl.chunks_mut(128).zip(br.chunks_mut(128)) {
            b.set_transport(Some(11_146.0 / 240_000.0));
            b.process(l, r);
        }
        assert_eq!(al, bl);
        assert_eq!(ar, br);
        assert_eq!(super::super::song_position(0.046_439_909), 11_146.0 / 240_000.0);
    }

    /// Null test of a synced LFO with the transport playing: `FILTER_PLAY=in.wav,out.wav`
    /// (`maschine-ref render --play --tempo 120`, 1024-sample blocks), the `sync1` settings
    /// of docs/fx-filter.md §7.
    #[test]
    #[ignore]
    fn synced_with_transport() {
        let Ok(v) = std::env::var("FILTER_PLAY") else { return };
        let (inp, out) = v.split_once(',').unwrap();
        let mut rd = hound::WavReader::open(inp).unwrap();
        let sr = rd.spec().sample_rate as f64;
        let x: Vec<f32> = rd.samples::<f32>().map(Result::unwrap).collect();
        let (mut l, mut r): (Vec<f32>, Vec<f32>) = (x.iter().step_by(2).copied().collect(), x.iter().skip(1).step_by(2).copied().collect());
        let mut f = Filter::new(sr);
        f.set(&params(&[("mode", 0.0), ("cutoff", 0.4), ("resonance", 0.3), ("amount", 0.8), ("source", 1.0), ("speed", 0.9)]));
        f.set_tempo(120.0);
        let blk: usize = std::env::var("FILTER_BLOCK").map_or(1024, |b| b.parse().unwrap());
        for (i, (a, b)) in l.chunks_mut(blk).zip(r.chunks_mut(blk)).enumerate() {
            f.set_transport(Some((i * blk) as f64 / sr * 120.0 / 60.0));
            f.process(a, b);
        }
        let spec = hound::WavSpec { channels: 2, sample_rate: sr as u32, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut w = hound::WavWriter::create(out, spec).unwrap();
        for (a, b) in l.iter().zip(&r) {
            w.write_sample(*a).unwrap();
            w.write_sample(*b).unwrap();
        }
        w.finalize().unwrap();
    }
}
