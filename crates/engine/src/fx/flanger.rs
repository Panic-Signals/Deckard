//! Maschine's Flanger (docs/fx-flanger.md): `RT::FlangerProcessor` → `FXCORE::FlangerCore`,
//! a compiled Reaktor-Core kernel. Two 4096-sample delay lines (cubic-interpolated reads)
//! with soft-clipped feedback taken after a ~45 Hz DC-blocking high-pass. A triangle LFO
//! (its rate from `LEGACY::SyncableLFOHelper`, free or tempo-synced) or an envelope
//! follower sweeps the delay from the Frequency's centre toward its minimum (Amount > 0)
//! or its maximum (Amount < 0). Centre, depth, feedback, dry and wet gains are smoothed
//! at a control rate of `round(sr/8820)` samples. All arithmetic follows the assembly
//! operation by operation (`f32`, `f64` where the kernel uses doubles, no fused
//! multiply-adds), so the output can match bit for bit.

use super::{Effect, param};

/// An `f32` constant as Maschine stores it.
const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// Delay-line length (the kernel masks indices with 0xfff).
const LEN: usize = 4096;
const MASK: u32 = 0xfff;
/// The longest read the kernel allows (samples).
const MAX_READ: f32 = c(0x457f_e000); // 4094
/// The smoothers' offset that keeps them off denormals (`0x2b2bff2ee48e0530`).
const TINY_D: f64 = 1e-100;
/// The high-pass state's offset (`0x219392ef`).
const TINY_F: f32 = c(0x2193_92ef); // 1e-18
/// The smoothers stop once within this of their targets (§2.4).
const SETTLED: f64 = 1e-4;
/// Feedback clip: clamp to ±3.36, then `x·(1 − |x|·0.1488)` (peaks at ±1.68).
const CLIP: f32 = c(0x4057_0a3d); // 3.36
const CLIP_K: f32 = c(0x3e18_6186); // 0.14880952
/// Delay (ms) at Frequency 100 % and 0 %.
const D_MIN: f32 = c(0x3dcd_badd); // 0.10045407
const D_MAX: f32 = c(0x4289_41fe); // 68.62889
/// About `1/(D_MAX − D_MIN)`: scales the sweep span for the rate-dependent reduction.
const SPAN_K: f32 = c(0x3c6f_154f); // 0.014592483
/// The follower's starting level.
const ENV_START: f32 = c(0x3dcc_cccd); // 0.1
/// Two entries of a log table the kernel interpolates at a constant position (as in
/// docs/fx-chorus.md §3.1); with −61 they give the smoothers' log2 decay.
const LOG_A: f32 = c(0x3e9e_defc); // 0.310295
const LOG_B: f32 = c(0x3e9f_7447); // 0.311434
const LOG_WA: f32 = c(0x3f75_d200); // 0.9602356
const LOG_WB: f32 = c(0x3d22_e000); // 0.039764404

/// Depth factor over `reduction·Hz` (0..10): fast LFOs sweep less of a wide span (§3.2).
const RATE_DEPTH: [f32; 10] = [1.0, 1.0, 1.0, c(0x3f59_999a), 0.75, c(0x3f0c_cccd), 0.5, c(0x3ee6_6666), c(0x3ecc_cccd), c(0x3eb3_3333)];
/// Per Invert (off, on): feedback sign, feedback-compensation factor, delay scale.
const FB_SIGN: [f32; 2] = [1.0, -1.0];
const KNEE_K: [f32; 2] = [c(0x3ea8_f5c3), c(0x3e99_999a)]; // 0.33, 0.3
const D_SCALE: [f32; 2] = [1.0, 0.5];

/// The LFO rates as `(numerator, denominator)` bars, slowest first.
const RATES: [(i32, i32); 11] = [(16, 1), (8, 1), (4, 1), (2, 1), (1, 1), (1, 2), (3, 8), (1, 4), (3, 16), (1, 8), (1, 16)];

/// `(int)(x ± 0.5)` in `f64`, the sign by `x < 0`: Maschine's round-half-away.
fn round(x: f32) -> i32 {
    (x as f64 + if x < 0.0 { -0.5 } else { 0.5 }) as i32
}

/// `2^n` for an integer `n`, built from the exponent bits.
fn pow2i(n: i32) -> f32 {
    f32::from_bits(0x3f80_0000i32.wrapping_add(n.wrapping_shl(23)) as u32)
}

/// `2^x` (docs/fx-chorus.md §3.1): a 1022-point table over one octave, linear
/// interpolation, and the integer part put into the exponent bits.
fn exp2_approx(x: f32) -> f32 {
    let n = {
        let h = if x >= 0.0 { 0.5 } else { -0.5 };
        (h + x as f64) as i32
    };
    let y = ((x - n as f32) + 0.5) * 1021.0;
    let k = round(y + -0.5);
    let fr = y - k as f32;
    let i = (k as u32 & 0x3ff) as usize;
    ((1.0 - fr) * exp2_entry(i) + fr * exp2_entry(i + 1)) * pow2i(n)
}

/// Entry `i` of the kernel's octave table (the Chorus's table): `2^(i/1021 − 0.5)` to 6
/// significant digits, the last entry √2 and seven near-ties rounded the other way.
fn exp2_entry(i: usize) -> f32 {
    match i {
        106 => return 0.759867,
        213 => return 0.817119,
        225 => return 0.823803,
        287 => return 0.859218,
        324 => return 0.881074,
        382 => return 0.91646,
        673 => return 1.11663,
        1022.. => return c(0x3fb5_04d5), // 1.41421
        _ => {}
    }
    let v = (i as f64 / 1021.0 - 0.5).exp2();
    let p = 10f64.powi(5 - v.log10().floor() as i32);
    ((v * p).round() / p) as f32
}

/// [`RATE_DEPTH`] at `x` (clamped to 0..10), linearly interpolated.
fn rate_depth(x: f32) -> f32 {
    let x = x.clamp(0.0, 10.0); // NaN stays NaN, as in the kernel
    let i = round(x + -0.5);
    let i0 = if (i as u32) < 9 { i as usize } else { 9 };
    let j = round(i as f32 + 1.0);
    let i1 = if (j as u32) < 9 { j as usize } else { 9 };
    RATE_DEPTH[i0] + (RATE_DEPTH[i1] - RATE_DEPTH[i0]) * (x - i as f32)
}

/// One cubic (Catmull-Rom) read of a delay line, `d` samples behind the sample just
/// written at `w`.
fn tap(buf: &[f32; LEN], w: u32, d: f32) -> f32 {
    let n = round(d + -0.5);
    let t = d - n as f32;
    let at = |k: u32| buf[(k & MASK) as usize];
    let a = at(w.wrapping_sub(n as u32));
    let b = at(w.wrapping_add(!(n as u32)));
    let cc = at(w.wrapping_add(1).wrapping_sub(n as u32));
    let e = at(w.wrapping_add(0xffe).wrapping_sub(n as u32));
    let c1 = (b - cc) * 0.5;
    let c2 = (e - a) * 0.5;
    let ab = a - b;
    let s = ab + c1;
    let q = ab + (c2 + s);
    a + t * (c1 + t * (t * q - (s + q)))
}

/// Input plus the soft-clipped feedback `v` (§3.4).
fn clip(x: f32, v: f32) -> f32 {
    let v = if CLIP <= v {
        CLIP
    } else if -CLIP <= v {
        v
    } else {
        -CLIP
    };
    x + v * (1.0 - v.abs() * CLIP_K)
}

/// A double-precision parameter smoother: target, current, coefficient, and the current
/// value as the kernel uses it (`f32`).
#[derive(Clone, Copy, Default)]
struct Smooth {
    t: f64,
    s: f64,
    c: f64,
    v: f32,
}

impl Smooth {
    fn step(&mut self) {
        self.s += self.c * ((self.t - self.s) + TINY_D);
        self.v = self.s as f32;
    }

    /// Within [`SETTLED`] of the target (the processor's per-block test).
    fn settled(&self) -> bool {
        (self.v as f64 - self.t).abs() <= SETTLED
    }
}

/// `NI::MASCOMMON::LEGACY::SyncableLFOHelper` over [`RATES`] (docs/fx-filter.md §4).
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
    const N: i32 = RATES.len() as i32;

    fn new() -> Lfo {
        // The processor sets the phase offset to 0.5.
        let mut l = Lfo { mode: 1, speed: 0.5, free: 0.0, beats: 0.0, out: 0.0, offset: 0.5, reset: None };
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
        let pos = (Lfo::N - 1) as f32 * self.speed;
        let i = round(pos - 0.5).clamp(0, Lfo::N - 2);
        let f = pos - i as f32;
        let s = (1.0 - f).mul_add(Lfo::ratio(i as usize), f * Lfo::ratio(i as usize + 1));
        1.0 / (s + s)
    }

    fn synced_beats(&self) -> f32 {
        let i = round((Lfo::N - 1) as f32 * self.speed).clamp(0, Lfo::N - 1);
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

    /// `switchToMode`.
    fn switch(&mut self, mode: i32) {
        if self.mode != mode {
            self.mode = mode;
            self.update();
        }
    }

    /// `newFrame`: the rate, and with the transport at `quarters` (playing), synced, the
    /// phase from the song position. Off (Env), the last rate stays.
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

/// `FXCORE::FlangerCore`'s inputs and state (§3).
struct Core {
    sr: f32,
    // Inputs, as the processor sets them (§2.2).
    in_fb: f32,
    in_invert: f32,
    in_mix: f32,
    in_freq: f32,
    in_env: f32,
    in_hz: f32,
    in_phase: f32,
    in_shape: f32,
    in_amount: f32,
    in_stereo: f32,
    /// Control ticks come every `period + 1` samples; samples since the last.
    ctr: i32,
    period: i32,
    /// Bits: centre (1), depth (2), feedback (4), dry (8), wet (0x10) still smoothing; any
    /// set → all five step.
    flags: u32,
    center: Smooth,
    depth: Smooth,
    fb: Smooth,
    dry: Smooth,
    wet: Smooth,
    /// Feedback: `fb(2 − fb)`, its sign and compensation factor (Invert), and the wet
    /// compensation `g` (§3.3).
    fb_curve: f32,
    fb_sign: f32,
    knee_k: f32,
    g: f32,
    mix_sq: f32,
    mix_curve: f32,
    /// The centre delay's room (ms) to the minimum and the maximum.
    d_lo: f32,
    d_hi: f32,
    amount: f32,
    amount_sq: f32,
    /// The sweep span (ms, signed), its reduction factor, whether the LFO is on, and
    /// their product.
    span: f32,
    span_red: f32,
    lfo_on: f32,
    red: f32,
    /// The rate (Hz), 1/sr, cycles per sample, and the LFO phase (−0.5..0.5).
    hz: f32,
    inv_sr: f32,
    inc: f32,
    phase: f32,
    env: bool,
    /// Envelope follower: level, samples since the last rise, hold (samples), fall per
    /// sample. `ms`: samples per ms (also the delay's ms → samples).
    level: f32,
    held: i32,
    hold: i32,
    fall: f32,
    ms: f32,
    /// Stereo: the right LFO's phase offset (cycles) and the right follower's scale.
    st_offset: f32,
    st_env: f32,
    /// Delay scale (Invert halves the delay).
    d_scale: f32,
    /// DC-blocking high-pass: coefficients, and per side its state and last output (the
    /// wet signal, also the feedback).
    hp_a: f32,
    hp_b: f32,
    z_l: f32,
    z_r: f32,
    out_l: f32,
    out_r: f32,
    widx: u32,
    buf_l: Box<[f32; LEN]>,
    buf_r: Box<[f32; LEN]>,
}

impl Core {
    /// `FlangerProcessor()`: a zeroed core with the processor's own input defaults.
    fn new(sr: f32) -> Core {
        Core { in_mix: 1.0, in_hz: 1.0, ..Core::zeroed(sr) }
    }

    fn zeroed(sr: f32) -> Core {
        Core {
            sr,
            in_fb: 0.0,
            in_invert: 0.0,
            in_mix: 0.0,
            in_freq: 0.0,
            in_env: 0.0,
            in_hz: 0.0,
            in_phase: 0.0,
            in_shape: 0.0,
            in_amount: 0.0,
            in_stereo: 0.0,
            ctr: 0,
            period: 0,
            flags: 0,
            center: Smooth::default(),
            depth: Smooth::default(),
            fb: Smooth::default(),
            dry: Smooth::default(),
            wet: Smooth::default(),
            fb_curve: 0.0,
            fb_sign: 0.0,
            knee_k: 0.0,
            g: 0.0,
            mix_sq: 0.0,
            mix_curve: 0.0,
            d_lo: 0.0,
            d_hi: 0.0,
            amount: 0.0,
            amount_sq: 0.0,
            span: 0.0,
            span_red: 0.0,
            lfo_on: 0.0,
            red: 0.0,
            hz: 0.0,
            inv_sr: 0.0,
            inc: 0.0,
            phase: 0.0,
            env: false,
            level: 0.0,
            held: 0,
            hold: 0,
            fall: 0.0,
            ms: 0.0,
            st_offset: 0.0,
            st_env: 0.0,
            d_scale: 0.0,
            hp_a: 0.0,
            hp_b: 0.0,
            z_l: 0.0,
            z_r: 0.0,
            out_l: 0.0,
            out_r: 0.0,
            widx: 0,
            buf_l: Box::new([0.0; LEN]),
            buf_r: Box::new([0.0; LEN]),
        }
    }

    // ---- Events (`FlangerProcessor::process*Parameter`, §2.2) ----

    /// The depth target: the span, reduced for fast LFOs over wide spans.
    fn depth_target(&mut self) {
        self.depth.t = (self.span * rate_depth(self.red * self.hz)) as f64;
    }

    /// The span from the centre's room and Amount (positive: toward the minimum delay,
    /// linear; negative: toward the maximum, squared), and its reduction factor.
    fn span(&mut self) {
        self.span = if self.amount > 0.0 { -(self.amount * self.d_lo) } else { -(self.amount_sq * self.d_hi) };
        let s = self.span.abs() * SPAN_K;
        self.span_red = if s > 1.0 {
            c(0x3f7f_ffef) // 0.999999
        } else if s < c(0x3e99_999a) {
            0.0
        } else {
            (s + c(0xbe99_999a)) * c(0x3fb6_db62) // (s − 0.3)·1.42857
        };
        self.red = self.span_red * self.lfo_on;
        self.depth_target();
    }

    /// `processEventInCtr`: Frequency → centre delay (ms).
    fn set_freq(&mut self, v: f32) {
        self.in_freq = v;
        let x = (v * (v.abs() * c(0x3ea8_f5c3) + c(0x3f2b_851f)) * 113.0 + 10.0) * c(0x3daa_a872);
        let n = round(x);
        let f = x - n as f32;
        let p = (((f * c(0x3d65_6042) + c(0x3e77_ced9)) * f + c(0x3f31_6873)) * f + 1.0) * pow2i(n);
        let d = if p < c(0x3a83_126f) { c(0x47ee_d800) } else { c(0x42f4_9375) / p };
        self.d_hi = D_MAX - d;
        self.d_lo = d - D_MIN;
        self.center.t = d as f64;
        self.span();
        self.flags |= 3;
    }

    fn set_amount(&mut self, v: f32) {
        self.in_amount = v;
        self.amount = v;
        self.amount_sq = if v < 0.0 { -(v * v) } else { v * v };
        self.span();
        self.flags |= 2;
    }

    fn set_shape(&mut self, v: f32) {
        self.in_shape = v;
        let u = (v + v) + -1.0;
        let s = 1.0 - u.abs();
        let (hold, fall) = if u <= c(0x3c23_d70a) {
            ((s * 2000.0 + 1000.0) * c(0x3dec_4ef9), c(0x4266_c51f))
        } else {
            (c(0x4138_9db3), (s * 16000.0 + 1000.0) * c(0x3dec_4ef9))
        };
        self.hold = round(hold * self.ms);
        let r = fall * self.ms;
        self.fall = if r > 1.0 { 1.0 / r } else { 1.0 };
    }

    fn set_stereo(&mut self, v: f32) {
        self.in_stereo = v;
        self.st_env = v * c(0xbca3_d70a) + 1.0;
        self.st_offset = (v * v) * 0.5;
    }

    /// The feedback target, the wet compensation, and the dry and wet targets (§3.3).
    fn feedback(&mut self) {
        let f = self.fb_curve * self.fb_sign;
        let a = f.abs();
        self.fb.t = f as f64;
        let k = if a > c(0x3f23_d70a) {
            let x = (a + c(0xbf23_d70a)) * c(0x4031_c726);
            (x * x) * self.knee_k
        } else {
            0.0
        };
        self.g = c(0x3f6b_851f) - k;
        self.wet.t = (self.g * self.mix_curve) as f64;
        self.dry.t = ((self.g + -1.0) * self.mix_sq + 1.0) as f64;
        self.flags |= 0x1c;
    }

    fn set_feedback(&mut self, v: f32) {
        self.in_fb = v;
        self.fb_curve = (2.0 - v) * v;
        self.feedback();
    }

    fn set_invert(&mut self, v: f32) {
        self.in_invert = v;
        let i = (round(v) != 0) as usize;
        self.knee_k = KNEE_K[i];
        self.fb_sign = FB_SIGN[i];
        self.d_scale = D_SCALE[i];
    }

    fn set_mix(&mut self, v: f32) {
        self.in_mix = v;
        self.mix_sq = v * v;
        self.dry.t = (self.mix_sq * (self.g + -1.0) + 1.0) as f64;
        self.mix_curve = (2.0 - v) * v;
        self.wet.t = (self.mix_curve * self.g) as f64;
        self.flags |= 0x18;
    }

    /// Source: the LFO (`v` 0) or the envelope follower (1).
    fn set_env(&mut self, v: f32) {
        self.in_env = v;
        self.env = round(v) & 1 != 0;
        self.lfo_on = (!self.env) as i32 as f32;
        self.red = self.span_red * self.lfo_on;
    }

    /// The LFO rate for the next block (`FlangerProcessor::process`, §2.4).
    fn set_rate(&mut self, hz: f32) {
        self.in_hz = hz;
        self.hz = hz;
        self.depth_target();
        self.flags |= 2;
        self.inc = hz * self.inv_sr;
    }

    // ---- resume / endInit (§2.1) ----

    /// `FlangerProcessor::resume` + `FlangerCore::endInit`: clear everything but the
    /// inputs, derive the state from them (smoothers at their targets), and run one
    /// sample on silence.
    fn resume(&mut self) {
        let old = std::mem::replace(self, Core::zeroed(self.sr));
        let sr = old.sr;

        // Control rate and the smoothers' coefficient (10 ms to −365 dB).
        let steps = round(sr / 8820.0);
        let ctl = if steps == 0 { 0 } else { round(sr) / steps };
        self.period = steps - 1;
        self.ms = sr / 1000.0;
        self.inv_sr = 1.0 / sr;
        let log = (LOG_A * LOG_WA + LOG_B * LOG_WB) + -61.0;
        let per = ((ctl as f32 as f64 / 1000.0) * 100.0 + -1.0).max(1.0);
        let coef = 1.0 - exp2_approx((log as f64 / per) as f32) as f64;
        for s in [&mut self.wet, &mut self.dry, &mut self.fb, &mut self.depth, &mut self.center] {
            s.c = coef;
        }
        self.level = ENV_START;

        // The DC blocker: a one-pole high-pass at min(0.494·sr, 45) Hz, the tangent by a
        // rational approximation.
        let w = (c(0x4049_0fd0) / sr) * (sr * c(0x3efc_ed91)).min(45.0);
        let x2 = w * w;
        let mut num = x2 * c(0xb2cd_4ca8) + c(0x3638_b94f);
        for k in [0xb950_0c03, 0x3c08_8885, 0xbe2a_aac1] {
            num = x2 * num + c(k);
        }
        num = w * (x2 * num + 1.0);
        let mut den = x2 * c(0xb48b_dad9) + c(0x37cf_b5a1);
        for k in [0xbab6_09bd, 0x3d2a_aa99, 0xbeff_f5e8] {
            den = x2 * den + c(k);
        }
        let t = num / (x2 * den + 1.0);
        self.hp_b = 1.0 / (t + 1.0);
        self.hp_a = t + -1.0;

        // The inputs, through the same formulas as the events.
        self.set_stereo(old.in_stereo);
        self.in_mix = old.in_mix;
        self.mix_curve = old.in_mix * (2.0 - old.in_mix);
        self.mix_sq = old.in_mix * old.in_mix;
        self.set_shape(old.in_shape);
        self.set_env(old.in_env);
        self.set_invert(old.in_invert);
        self.in_fb = old.in_fb;
        self.fb_curve = old.in_fb * (2.0 - old.in_fb);
        self.feedback();
        self.in_amount = old.in_amount;
        self.amount = old.in_amount;
        self.amount_sq = if old.in_amount < 0.0 { -(old.in_amount * old.in_amount) } else { old.in_amount * old.in_amount };
        self.in_hz = old.in_hz;
        self.hz = old.in_hz;
        self.inc = old.in_hz * self.inv_sr;
        self.set_freq(old.in_freq);
        self.in_phase = old.in_phase;
        self.phase = old.in_phase;

        // Smoothers start at their targets; endInit's own sample (silence in, output
        // dropped) takes one control step and advances the LFO once.
        for s in [&mut self.wet, &mut self.dry, &mut self.fb, &mut self.depth, &mut self.center] {
            s.s = s.t;
        }
        self.flags = 0x1f;
        self.ctr = self.period;
        self.tick(0.0, 0.0);
    }

    // ---- processAudio (§3) ----

    fn tick(&mut self, xl: f32, xr: f32) -> (f32, f32) {
        let w = self.widx;
        self.widx = (w + 1) & MASK;
        if self.ctr < self.period {
            self.ctr += 1;
        } else {
            self.ctr = 0;
            if self.flags >= 1 {
                self.wet.step();
                self.dry.step();
                self.fb.step();
                self.depth.step();
                self.center.step();
            }
        }
        // Feedback (the high-passed wet signal of the last sample), soft-clipped, into
        // the lines.
        self.buf_r[w as usize] = clip(xr, self.out_r * self.fb.v);
        let fl = self.out_l * self.fb.v;

        let (ml, mr) = if !self.env {
            // Triangle LFO, 0..1; the right side runs `st_offset` cycles ahead.
            let p = self.phase;
            let q = p + self.st_offset;
            let q = q - round(q) as f32;
            let tl = (p * 0.5).abs();
            let tl = tl + tl;
            let tr = (q * 0.5).abs();
            let tr = tr + tr;
            let n = p + self.inc;
            self.phase = n - round(n) as f32;
            (tl + tl, tr + tr)
        } else {
            // Envelope follower: instant rise, hold, then a linear fall.
            let lvl = if xl.abs() > xr.abs() { xl.abs() } else { xr.abs() };
            if lvl > self.level {
                self.held = 0;
                self.level = lvl;
            } else {
                let h = self.held;
                self.held += 1;
                if self.hold <= h {
                    self.level = (self.level - self.fall).max(0.0);
                }
            }
            let e = (self.level + c(0x3586_37bd)).min(1.0);
            (e, self.st_env * e)
        };
        self.buf_l[w as usize] = clip(xl, fl);

        let yl0 = xl * self.dry.v;
        let yr0 = xr * self.dry.v;
        let dr = (self.center.v + mr * self.depth.v) * self.d_scale * self.ms;
        let dl = (self.center.v + ml * self.depth.v) * self.d_scale * self.ms;
        let read = |d: f32| d.clamp(0.0, MAX_READ);
        let h = self.hp_b * (tap(&self.buf_r, w, read(dr)) - self.z_r * self.hp_a);
        let z = self.z_r;
        self.z_r = h + TINY_F;
        self.out_r = h - z;
        let yr = yr0 + self.wet.v * self.out_r;
        let h = self.hp_b * (tap(&self.buf_l, w, read(dl)) - self.z_l * self.hp_a);
        let z = self.z_l;
        self.z_l = h + TINY_F;
        self.out_l = h - z;
        let yl = yl0 + self.wet.v * self.out_l;
        (yl, yr)
    }
}

/// Field names in constructor (event) order, with defaults: Frequency, Amount, Source,
/// Speed, Shape, Stereo, Feedback, Invert, Mix.
const PARAMS: [(&str, f64); 9] = [
    ("frequency", 0.5),
    ("amount", 0.75),
    ("source", 0.0),
    ("speed", 0.5),
    ("shape", 0.5),
    ("stereo", 0.5),
    ("feedback", 0.5),
    ("invert", 0.0),
    ("mix", 0.5),
];

/// Maschine's processing block (samples): the LFO rate and the smoothing stop update
/// once per block (§2.4).
const BLOCK: usize = 128;

/// The Flanger effect: the parameters go to the core as events; the LFO helper gives
/// it a rate per block.
pub struct Flanger {
    core: Core,
    lfo: Lfo,
    /// The Speed as the processor last stored it.
    speed: f32,
    bpm: f32,
    /// The song position (quarters) at the next block, while the transport plays.
    quarters: Option<f64>,
    values: Option<[f32; 9]>,
    block: usize,
}

impl Flanger {
    pub fn new(sr: f64) -> Flanger {
        Flanger { core: Core::new(sr as f32), lfo: Lfo::new(), speed: 0.0, bpm: 120.0, quarters: None, values: None, block: BLOCK }
    }

    /// The processor's handler for parameter `k` of [`PARAMS`].
    fn apply(&mut self, k: usize, v: f32) {
        let core = &mut self.core;
        match k {
            0 => core.set_freq(v),
            1 => core.set_amount(v),
            2 => {
                // LFO → free, LFO Sync → synced, Env → off.
                let s = v as i64;
                let mode = match s {
                    0 => 1,
                    1 => 2,
                    _ => 0,
                };
                if mode != 0 {
                    self.lfo.speed = self.speed;
                    self.lfo.update();
                }
                self.lfo.switch(mode);
                core.set_env(if s == 2 { 1.0 } else { 0.0 });
                core.depth_target();
                core.flags |= 2;
            }
            3 => {
                self.speed = v;
                self.lfo.speed = v;
                self.lfo.update();
            }
            4 => core.set_shape(v),
            5 => core.set_stereo(v),
            6 => core.set_feedback(v),
            7 => {
                core.set_invert(if v != 0.0 { 1.0 } else { 0.0 });
                core.feedback();
            }
            _ => core.set_mix(v),
        }
    }

    /// `FlangerProcessor::process` for one block: the LFO rate, the smoothing stop, then
    /// the core per sample.
    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.lfo.frame(self.bpm, self.quarters);
        let core = &mut self.core;
        core.set_rate(self.lfo.out);
        if let Some(p) = self.lfo.reset {
            core.in_phase = p;
            core.phase = p;
        }
        let mut flags = core.flags;
        for (s, b) in [(&core.center, 1), (&core.depth, 2), (&core.fb, 4), (&core.dry, 8), (&core.wet, 0x10)] {
            if s.settled() {
                flags &= !b;
            }
        }
        core.flags = flags;
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            (*a, *b) = core.tick(*a, *b);
        }
    }
}

impl Effect for Flanger {
    fn set(&mut self, params: &[(String, f64)]) {
        let mut new = [0f32; 9];
        for (k, (name, def)) in PARAMS.iter().enumerate() {
            new[k] = param(params, name, *def) as f32;
        }
        match self.values {
            None => {
                // The parameters reach the processor in constructor order, then it
                // resumes.
                for (k, v) in new.iter().enumerate() {
                    self.apply(k, *v);
                }
                self.core.resume();
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

    fn set_tempo(&mut self, bpm: f64) {
        self.bpm = bpm as f32;
    }

    fn set_transport(&mut self, quarters: Option<f64>) {
        self.quarters = quarters;
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let b = self.block.max(1);
        for (x, y) in l.chunks_mut(b).zip(r.chunks_mut(b)) {
            self.run(x, y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(kv: &[(&str, f64)]) -> Vec<(String, f64)> {
        kv.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn noise(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i * 7919 + 13) % 1000) as f32 / 1000.0 - 0.5).collect()
    }

    fn run(fx: &mut Flanger, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut a, mut b) = (l.to_vec(), r.to_vec());
        for (x, y) in a.chunks_mut(512).zip(b.chunks_mut(512)) {
            fx.process(x, y);
        }
        (a, b)
    }

    #[test]
    fn exp2_approx_is_two_to_the_x() {
        for x in [-3.0f32, -1.0, -0.25, 0.0, 0.4, 2.0] {
            assert!((exp2_approx(x) / x.exp2() - 1.0).abs() < 1e-5, "{x}");
        }
    }

    #[test]
    fn frequency_spans_the_delay_range() {
        let mut c = Core::new(44_100.0);
        c.set_freq(0.0);
        assert!((c.center.t - 68.6).abs() < 0.1, "{}", c.center.t);
        c.set_freq(1.0);
        assert!((c.center.t - 0.1).abs() < 0.01, "{}", c.center.t);
        assert_eq!(rate_depth(0.0), 1.0);
        assert_eq!(rate_depth(10.0), RATE_DEPTH[9]);
    }

    #[test]
    fn lfo_rates() {
        let mut l = Lfo::new();
        assert_eq!(l.free, 1.0); // Speed 50 %: 1/2 bar → 1 Hz
        l.speed = 0.0;
        l.update();
        assert_eq!(l.free, 1.0 / 32.0);
        l.speed = 1.0;
        l.update();
        assert_eq!(l.free, 8.0);
        l.switch(2);
        l.frame(120.0, None);
        assert!((l.out - 8.0).abs() < 1e-5, "{}", l.out); // 1/16 bar at 120 BPM
    }

    #[test]
    fn defaults_flange_the_input() {
        let mut fx = Flanger::new(44_100.0);
        fx.set(&[]);
        let x = noise(44_100);
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        let diff: f32 = l.iter().zip(&x).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 100.0);
        // Stereo 50 %: the right LFO runs ahead, so the sides differ.
        assert!(l.iter().zip(&r).any(|(a, b)| a != b));
    }

    #[test]
    fn dry_mix_is_the_identity() {
        let mut fx = Flanger::new(48_000.0);
        fx.set(&p(&[("mix", 0.0), ("feedback", 1.0)]));
        let x = noise(20_000);
        let (l, r) = run(&mut fx, &x, &x);
        assert_eq!(l, x);
        assert_eq!(r, x);
    }

    #[test]
    fn channels_are_independent_and_silence_stays_silent() {
        for src in [0.0, 2.0] {
            let mut fx = Flanger::new(44_100.0);
            fx.set(&p(&[("source", src), ("mix", 1.0), ("feedback", 0.9)]));
            let x = noise(30_000);
            let z = vec![0.0; x.len()];
            let (l, r) = run(&mut fx, &x, &z);
            assert!(l.iter().any(|v| *v != 0.0));
            // (Only the high-pass state's 1e-18 offset leaks out.)
            assert!(r.iter().all(|v| v.abs() < 1e-15), "{src}");
        }
    }

    #[test]
    fn set_keeps_state_and_smooths() {
        let x = noise(40_960);
        let mut a = Flanger::new(44_100.0);
        a.set(&p(&[("amount", 0.4)]));
        let (l1, _) = run(&mut a, &x[..10_240], &x[..10_240]);
        // Re-sending the same values changes nothing.
        a.set(&p(&[("amount", 0.4)]));
        let (l2, _) = run(&mut a, &x[10_240..], &x[10_240..]);
        let mut b = Flanger::new(44_100.0);
        b.set(&p(&[("amount", 0.4)]));
        let (lb, _) = run(&mut b, &x, &x);
        assert_eq!([l1, l2].concat(), lb);

        // Mix 0 → 1 glides over several control steps; at Mix 100 % the dry gain is the
        // wet compensation g, not 0.
        let mut c = Flanger::new(44_100.0);
        c.set(&p(&[("mix", 0.0)]));
        run(&mut c, &x[..1024], &x[..1024]);
        c.set(&p(&[("mix", 1.0)]));
        run(&mut c, &x[..5], &x[..5]);
        let (d, g) = (c.core.dry.v, c.core.g);
        assert!(d > g && d < 1.0, "{d} {g}");
        run(&mut c, &x, &x);
        assert!((c.core.dry.v - g).abs() < 1e-3, "{}", c.core.dry.v);
    }

    #[test]
    fn loud_input_with_full_feedback_stays_bounded() {
        for inv in [0.0, 1.0] {
            let mut fx = Flanger::new(96_000.0);
            fx.set(&p(&[("feedback", 1.0), ("invert", inv), ("mix", 1.0), ("frequency", 0.0), ("amount", -1.0)]));
            let x: Vec<f32> = noise(200_000).iter().map(|v| v * 8.0).collect();
            let (l, r) = run(&mut fx, &x, &x);
            assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 50.0));
        }
    }
    /// Null test of a synced LFO with the transport playing: `FLANGER_PLAY=in.wav,out.wav`
    /// (`maschine-ref render --play --tempo 120`), the `syncP` settings of
    /// docs/fx-flanger.md §5; `FLANGER_QOFF` offsets the song position (quarters).
    #[test]
    #[ignore]
    fn synced_with_transport() {
        let Ok(v) = std::env::var("FLANGER_PLAY") else { return };
        let (inp, out) = v.split_once(',').unwrap();
        let mut rd = hound::WavReader::open(inp).unwrap();
        let sr = rd.spec().sample_rate as f64;
        let x: Vec<f32> = rd.samples::<f32>().map(Result::unwrap).collect();
        let (mut l, mut r): (Vec<f32>, Vec<f32>) = (x.iter().step_by(2).copied().collect(), x.iter().skip(1).step_by(2).copied().collect());
        let mut f = Flanger::new(sr);
        let kv = std::env::var("FLANGER_PARAMS").unwrap_or("frequency=0.2,source=1,amount=0.9,speed=0.7,feedback=0.55,mix=1".into());
        let kv: Vec<(String, f64)> =
            kv.split(',').map(|e| e.split_once('=').unwrap()).map(|(k, v)| (k.to_string(), v.parse().unwrap())).collect();
        f.set(&kv);
        f.set_tempo(120.0);
        let off: f64 = std::env::var("FLANGER_QOFF").map_or(0.0, |b| b.parse().unwrap());
        let blk: usize = std::env::var("FLANGER_HOST").map_or(1024, |b| b.parse().unwrap());
        for (i, (a, b)) in l.chunks_mut(blk).zip(r.chunks_mut(blk)).enumerate() {
            f.set_transport(Some((i * blk) as f64 / sr * 2.0 + off));
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
