//! Maschine's Phaser (docs/fx-phaser.md): `RT::PhaserProcessor` → `FXCORE::PhaserCore`, a
//! compiled Reaktor-Core kernel. Per channel, a chain of four (8 Pole) or three second-order
//! allpass sections sits in a zero-delay feedback loop: the loop's node is solved
//! implicitly, through a first-order allpass, a ~92 Hz high-pass and a soft clipper. The
//! output mixes the dry input with that node and runs it through a two-pole low-pass. A
//! triangle LFO (its rate from `LEGACY::SyncableLFOHelper`, free or tempo-synced) or an
//! envelope follower moves the allpass frequency; Stereo offsets the right channel. Controls
//! are smoothed and the coefficients recomputed every `round(sr/11025)` samples. All
//! arithmetic follows the assembly operation by operation (`f32`, `f64` where the kernel
//! uses doubles, no fused multiply-adds), so the output can match bit for bit.

use super::{Effect, param};
use std::sync::OnceLock;

/// An `f32` constant as Maschine stores it.
const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// The kernel's denormal offset (`0x219392ef`).
const D: f32 = c(0x2193_92ef); // 1e-18
/// The smoothers' offset (`0x2b2bff2ee48e0530`).
const TINY_D: f64 = 1e-100;
/// The smoothers stop once within this of their targets (§3).
const SETTLED: f64 = 1e-4;
/// `1/12`: semitones to octaves.
const OCT: f32 = c(0x3daa_a880); // 0.0833292
/// MIDI note 0 in Hz.
const NOTE0: f32 = c(0x4102_d6b6); // 8.17742
/// The LFO smoother's scale (with 32 and `44100/control rate`, per Hz of LFO rate).
const SLEW_K: f32 = c(0x38da_f153); // 1.044e-4
/// The feedback clipper: clamp to ±6.666, then `x·(1 − |x|·0.0750075)`.
const CLIP: f32 = c(0x40d5_4fdf); // 6.666
const CLIP_K: f32 = c(0x3d99_9d89); // 0.0750075
/// Gain of the high-passed part added in the feedback loop.
const HP_K: f32 = c(0xbea5_1098); // −0.32239223
/// The feedback high-pass frequency (Hz).
const HP_HZ: f32 = c(0x42b8_a6f4); // 92.32608

/// The output level by `fbv·10` (§4.3), interpolated.
const LEVEL: [f32; 12] = [
    16.0,
    6.0,
    c(0x4053_3333), // 3.3
    c(0x400c_cccd), // 2.2
    c(0x3fd4_7ae1), // 1.66
    c(0x3f9c_28f6), // 1.22
    1.0,
    0.75,
    c(0x3f35_c28f), // 0.71
    c(0x3f0a_3d71), // 0.54
    c(0x3edc_28f6), // 0.43
    c(0x3edc_28f6),
];

/// The LFO rates as `(numerator, denominator)` bars, slowest first (§2).
const RATES: [(i32, i32); 11] = [(16, 1), (8, 1), (4, 1), (2, 1), (1, 1), (1, 2), (3, 8), (1, 4), (3, 16), (1, 8), (1, 16)];

/// `(int)(x ± 0.5)` in `f64`, the sign by `x < 0`: Maschine's round-half-away.
fn round(x: f32) -> i32 {
    (x as f64 + if x < 0.0 { -0.5 } else { 0.5 }) as i32
}

/// `2^n` for an integer `n`, built from the exponent bits.
fn pow2i(n: i32) -> f32 {
    f32::from_bits(0x3f80_0000i32.wrapping_add(n.wrapping_shl(23)) as u32)
}

/// `PhaserCore`'s table: `2^(i/1021 − 0.5)` for `i` in 0..=1021 as 6-digit decimals,
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
    let i = round(f + -0.5);
    let k = (i as u32 & 0x3ff) as usize;
    let t = f - i as f32;
    let tab = exp2_table();
    ((1.0 - t) * tab[k] + t * tab[k + 1]) * pow2i(n)
}

/// The kernel's sine and cosine polynomials: `(w·(1 − w²/6 + …), 1 − w²/2 + …)`.
fn sin_cos(w: f32) -> (f32, f32) {
    let w2 = w * w;
    let mut s = w2 * c(0xb2cd_4ca8) + c(0x3638_b94f);
    s = w2 * s + c(0xb950_0c03);
    s = w2 * s + c(0x3c08_8885);
    s = w2 * s + c(0xbe2a_aac1);
    s = w2 * s + 1.0;
    let mut q = w2 * c(0xb48b_dad9) + c(0x37cf_b5a1);
    q = w2 * q + c(0xbab6_09bd);
    q = w2 * q + c(0x3d2a_aa99);
    q = w2 * q + c(0xbeff_f5e8);
    (w * s, w2 * q + 1.0)
}

/// `NI::MASCOMMON::LEGACY::SyncableLFOHelper` over [`RATES`] (docs/fx-filter.md §4).
struct Lfo {
    /// 0 off (Envelope), 1 free, 2 synced.
    mode: i32,
    speed: f32,
    /// The free rate (Hz), and the synced cycle (beats).
    free: f32,
    beats: f32,
    /// The rate for this block (Hz).
    out: f32,
    /// Phase (cycles) added to the song position when synced: 0.5 for the Phaser.
    offset: f32,
    /// The phase the LFO restarts at this block, synced with the transport running.
    reset: Option<f32>,
}

impl Lfo {
    fn new() -> Lfo {
        Lfo { mode: 1, speed: 0.0, free: 0.0, beats: 0.0, out: 0.0, offset: 0.5, reset: None }
    }

    fn ratio(i: usize) -> f32 {
        RATES[i].0 as f32 / RATES[i].1 as f32
    }

    /// Between the two nearest rates: `1 / (2·bars)` Hz.
    fn free_rate(&self) -> f32 {
        let n = RATES.len() as i32;
        let pos = self.speed * (n - 1) as f32;
        let i = round(pos + -0.5).max(0).min(n - 2);
        let f = pos - i as f32;
        let s = (1.0 - f).mul_add(Lfo::ratio(i as usize), f * Lfo::ratio(i as usize + 1));
        1.0 / (s + s)
    }

    fn synced_beats(&self) -> f32 {
        let n = RATES.len() as i32;
        let i = round(self.speed * (n - 1) as f32).max(0).min(n - 1);
        Lfo::ratio(i as usize) * 4.0
    }

    /// `setSpeed`: the rate of the current mode.
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

/// One channel's allpass chain, feedback loop and output filter (§4.4–4.5).
#[derive(Default, Clone, Copy)]
struct Chan {
    /// The allpass sections' coefficients: `b0` (direct), `b0·0`, and the taps on the
    /// section input and output, one and two samples back.
    b0: f32,
    b0z: f32,
    c1: f32,
    c2: f32,
    c3: f32,
    c4: f32,
    /// `1 − Feedback·a·b0^n`: the loop's implicit-solve denominator.
    den: f32,
    /// Node values (+D): the clipped loop signal, then each section's output; and the
    /// same one sample earlier (+D again).
    n: [f32; 5],
    d: [f32; 5],
    /// The first-order allpass in the feedback path.
    z: f32,
    /// The feedback high-pass.
    hp: f32,
    /// The output low-pass (two one-poles).
    lp1: f32,
    lp2: f32,
}

impl Chan {
    /// The allpass coefficients for `w = 2π·f/sr`.
    fn coefs(&mut self, w: f32) {
        let (s, co) = sin_cos(w);
        let m = 1.0 - s;
        let p = s + 1.0;
        let inv = 1.0 / p;
        self.b0 = m * inv;
        self.c4 = -(m * inv);
        let k = co * -2.0;
        self.c1 = k * inv;
        self.c3 = -(k * inv);
        self.b0z = self.b0 * 0.0;
        self.c2 = p * inv;
    }
}

/// `FXCORE::PhaserCore`: inputs, smoothed controls, modulators and the two channels.
#[derive(Default)]
struct Core {
    sr: f32,
    // Inputs as the processor last set them (§3).
    in_fb: f32,
    in_mix: f32,
    in_eight: f32,
    in_freq: f32,
    in_env: f32,
    in_shape: f32,
    in_amount: f32,
    in_stereo: f32,
    /// Control ticks come every `period + 1` samples; samples since the last.
    period: i32,
    counter: i32,
    /// Bits: frequency (1), amount (2), feedback (4), dry (8), wet (16), low-pass (32)
    /// still smoothing; any → all do.
    flags: u32,
    /// The smoothers' coefficient (`f64`), targets and current values.
    coef: f64,
    lpf_t: f64,
    lpf_c: f64,
    wet_t: f64,
    wet_c: f64,
    dry_t: f64,
    dry_c: f64,
    fb_t: f64,
    fb_c: f64,
    amt_t: f64,
    amt_c: f64,
    freq_t: f64,
    freq_c: f64,
    /// The smoothed values as `f32`.
    wet: f32,
    dry: f32,
    fb: f32,
    amt: f32,
    freq: f32,
    // Derived from the parameters.
    fbc: f32,
    mix: f32,
    mixc: f32,
    level: f32,
    dry_k: f32,
    eight: bool,
    env_mode: bool,
    pitch: f32,
    amt87: f32,
    amt_idx: usize,
    span: [f32; 2],
    st_gain: f32,
    st_off: f32,
    // Constants of the sample rate.
    ctl_ms: f32,
    inv_ctl: f32,
    k44: f32,
    lo_hz: f32,
    hi_hz: f32,
    w_k: f32,
    lpf_max: f32,
    pi_sr: f32,
    a: f32,
    aa: f32,
    hp_g: f32,
    hp_inv: f32,
    hp_m1: f32,
    // LFO.
    inc: f32,
    phase: f32,
    lfo1: f32,
    lfo2: f32,
    slew: f32,
    // Envelope follower.
    env: f32,
    peak: f32,
    hold: i32,
    hold_ctr: i32,
    decay: f32,
    env_c: f32,
    // Output low-pass.
    g: f32,
    g_m1: f32,
    g_inv: f32,
    chans: [Chan; 2],
}

impl Core {
    /// `processFloatParameter` 0: Frequency, the centre pitch 37..124.
    fn set_freq(&mut self, v: f32) {
        self.in_freq = v;
        self.pitch = v * 87.0 + 37.0;
        self.update_span(self.pitch + self.amt87);
        self.freq_t = self.pitch as f64;
        self.amt_t = self.span[self.amt_idx] as f64;
        self.flags |= 3;
    }

    /// `processFloatParameter` 1: Amount (bipolar, squared, ×87 semitones).
    fn set_amount(&mut self, v: f32) {
        self.in_amount = v;
        let sq = if v < 0.0 { -(v * v) } else { v * v };
        self.amt87 = sq * 87.0;
        self.update_span(self.amt87 + self.pitch);
        self.amt_idx = (sq > 0.0) as usize;
        self.amt_t = self.span[self.amt_idx] as f64;
        self.flags |= 2;
    }

    /// The modulation's depth down and up from the centre, limited to pitch 37..124.
    fn update_span(&mut self, f: f32) {
        let lo = if 37.0 > f { 37.0 } else { f };
        let hi = if f > 124.0 { 124.0 } else { f };
        self.span = [lo - self.pitch, hi - self.pitch];
    }

    /// `processFloatParameter` 3: Shape, the envelope follower's hold and decay.
    fn set_shape(&mut self, v: f32) {
        self.in_shape = v;
        let x = (v + v) + -1.0;
        let y = 1.0 - x.abs();
        let (hold, rel) = if x > c(0x3c23_d70a) {
            (c(0x4138_9db3), (y * 16000.0 + 1000.0) * c(0x3dec_4ef9))
        } else {
            ((y * 2000.0 + 1000.0) * c(0x3dec_4ef9), c(0x4266_c51f))
        };
        self.hold = round(hold * self.ctl_ms);
        let r = rel * self.ctl_ms;
        self.decay = if r > 1.0 { 1.0 / r } else { 1.0 };
    }

    /// `processFloatParameter` 4: Stereo.
    fn set_stereo(&mut self, v: f32) {
        self.in_stereo = v;
        self.st_gain = v * c(0xbd4c_cccd) + 1.0;
        self.st_off = (v * v) * 0.5;
    }

    /// The output level for feedback `fbv` and the dry gain derived from it (§4.3).
    fn level_table(&mut self, fbv: f32) {
        let x = fbv * 10.0;
        let i = round(x + -0.5);
        let i0 = if (i as u32) < 11 { i as usize } else { 11 };
        let i1 = if (i.wrapping_add(1) as u32) >= 11 { 11 } else { (i + 1) as usize };
        let f = x - i as f32;
        self.level = LEVEL[i0] + f * (LEVEL[i1] - LEVEL[i0]);
        self.dry_k = (fbv + -1.0) * self.level + -1.0;
    }

    /// `processFloatParameter` 5: Feedback.
    fn set_feedback(&mut self, v: f32) {
        self.in_fb = v;
        self.fbc = (2.0 - v) * v;
        self.lpf_t = ((self.fbc * self.mix) * -12400.0 + 22000.0) as f64;
        let fbv = self.fbc * c(0x3f6a_7efa) + c(0x3d4c_cccd);
        self.level_table(fbv);
        self.wet_t = (self.mixc * self.level + 0.0) as f64;
        self.dry_t = (self.mixc * self.dry_k + 1.0) as f64;
        self.fb_t = fbv as f64;
        self.flags |= 0x3c;
    }

    /// `processFloatParameter` 6: Mix.
    fn set_mix(&mut self, v: f32) {
        self.lpf_t = ((self.fbc * v) * -12400.0 + 22000.0) as f64;
        self.in_mix = v;
        self.mix = v;
        self.mixc = (2.0 - v) * v;
        self.wet_t = (self.mixc * self.level + 0.0) as f64;
        self.dry_t = (self.mixc * self.dry_k + 1.0) as f64;
        self.flags |= 0x38;
    }

    /// `processEnumParameter` 0 (the core's part): Source Envelope or not.
    fn set_env(&mut self, env: bool) {
        self.in_env = if env { 1.0 } else { 0.0 };
        self.env_mode = (self.in_env as f64 + 0.5) as i32 & 1 == 1;
    }

    /// `processBoolParameter` 0: 8 Pole.
    fn set_eight(&mut self, on: bool) {
        self.in_eight = if on { 1.0 } else { 0.0 };
        self.eight = round(round(self.in_eight) as f32) > 0;
    }

    /// The LFO's rate for the next block (`PhaserProcessor::process`, §3).
    fn set_lfo_rate(&mut self, hz: f32) {
        let s = (hz * 32.0) * SLEW_K;
        self.slew = s * self.k44;
        self.inc = hz * self.inv_ctl;
    }

    /// `PhaserProcessor::resume` + `PhaserCore::endInit` (§5): clear everything but the
    /// inputs, derive every value from them, and run one control tick and one sample on
    /// silence.
    fn resume(&mut self) {
        *self = Core {
            sr: self.sr,
            in_fb: self.in_fb,
            in_mix: self.in_mix,
            in_eight: self.in_eight,
            in_freq: self.in_freq,
            in_env: self.in_env,
            in_shape: self.in_shape,
            in_amount: self.in_amount,
            in_stereo: self.in_stereo,
            ..Core::default()
        };
        let sr = self.sr;
        self.pi_sr = c(0x4049_0fd0) / sr; // 3.14159
        self.lo_hz = sr / 24576.0;
        self.hi_hz = sr / 2.125;
        self.w_k = c(0x40c9_0fe4) / sr; // 6.28319
        self.lpf_max = sr * c(0x3efc_ed91); // 0.494
        let steps = round(sr / 11025.0);
        let ctl = if steps == 0 { 0 } else { round(sr) / steps };
        let ctl_f = ctl as f32;
        self.ctl_ms = ctl_f / 1000.0;
        let q = sr / 88200.0;
        self.a = (1.0 - q) / (q + 1.0);
        self.aa = 1.0 - self.a * self.a;
        self.set_stereo(self.in_stereo);
        self.pitch = self.in_freq * 87.0 + 37.0;
        self.freq_t = self.pitch as f64;
        self.freq_c = self.freq_t;
        self.period = steps - 1;
        self.env_mode = round(self.in_env) & 1 == 1;
        self.k44 = 44100.0 / ctl_f;
        self.inv_ctl = 1.0 / ctl_f;
        self.set_shape(self.in_shape);
        self.mix = self.in_mix;
        self.mixc = self.in_mix * (2.0 - self.in_mix);
        self.fbc = self.in_fb * (2.0 - self.in_fb);
        self.peak = c(0x3dcc_cccd); // 0.1
        // The smoothers' coefficient: 2^−60.69 after 100 ms.
        let span = ((ctl_f as f64 / 1000.0) * 100.0 + -1.0).max(1.0);
        let log = (c(0x3e9e_defc) * c(0x3f75_d200) + c(0x3e9f_7447) * c(0x3d22_e000)) + -61.0;
        self.coef = 1.0 - exp2_lookup((log as f64 / span) as f32) as f64;
        let e = ctl_f * c(0x3b03_126f); // 0.002: 2 ms
        self.env_c = if e < 1.0 { c(0x3f31_7215) } else { c(0x3f31_7215) / e };
        self.counter = steps;
        self.set_lfo_rate(0.0);
        self.lpf_t = ((self.fbc * self.mix) * -12400.0 + 22000.0) as f64;
        self.lpf_c = self.lpf_t;
        let fbv = self.fbc * c(0x3f6a_7efa) + c(0x3d4c_cccd);
        self.level_table(fbv);
        self.wet_t = (self.mixc * self.level + 0.0) as f64;
        self.wet_c = self.wet_t;
        self.dry_t = (self.mixc * self.dry_k + 1.0) as f64;
        self.dry_c = self.dry_t;
        self.fb_t = fbv as f64;
        self.fb_c = self.fb_t;
        // The feedback high-pass at min(sr, 92.3 Hz): g = tan(w) as a series.
        let hp = if sr > HP_HZ { HP_HZ } else { sr };
        let w = self.pi_sr * if hp < 0.0 { 0.0 } else { hp };
        let w2 = w * w;
        let w3 = w * w2;
        let g = w + (w3 * c(0x3eaa_aa9f) + (w2 * w3) * c(0x3e08_8872));
        self.hp_g = g;
        self.hp_inv = 1.0 / (g + 1.0);
        self.hp_m1 = g + -1.0;
        self.set_amount(self.in_amount);
        self.amt_c = self.amt_t;
        self.eight = round(round(self.in_eight) as f32) > 0;
        // One control tick and one silent sample.
        self.flags = 1;
        self.tick(0.0, 0.0);
        self.flags = 0x3f;
    }

    /// The control-rate part of `processAudio` (§4.1–4.3).
    fn control(&mut self, xl: f32, xr: f32) {
        if self.flags as i32 >= 1 {
            let k = self.coef;
            self.lpf_c += k * ((self.lpf_t - self.lpf_c) + TINY_D);
            let lpf = self.lpf_c as f32;
            self.wet_c += k * ((self.wet_t - self.wet_c) + TINY_D);
            self.wet = self.wet_c as f32;
            self.dry_c += k * ((self.dry_t - self.dry_c) + TINY_D);
            self.dry = self.dry_c as f32;
            self.fb_c += k * ((self.fb_t - self.fb_c) + TINY_D);
            self.fb = self.fb_c as f32;
            self.amt_c += k * ((self.amt_t - self.amt_c) + TINY_D);
            self.amt = self.amt_c as f32;
            self.freq_c += k * ((self.freq_t - self.freq_c) + TINY_D);
            self.freq = self.freq_c as f32;
            // The output low-pass: g = tan(π·f/sr) as sin/cos.
            let mut f = if lpf < 0.0 { 0.0 } else { lpf };
            if f > self.lpf_max {
                f = self.lpf_max;
            }
            let (s, co) = sin_cos(f * self.pi_sr);
            self.g = s / co;
            self.g_inv = 1.0 / (self.g + 1.0);
            self.g_m1 = self.g + -1.0;
        }
        // Modulation for the left (m1) and right (m2) channels, 0..1.
        let (m1, m2);
        if !self.env_mode {
            // Triangles from the phase (−0.5..0.5), the right one offset by Stereo.
            let ph = self.phase;
            let mut t = (ph * 0.5).abs();
            t = t + t;
            t = t + t;
            self.lfo1 += self.slew * ((t - self.lfo1) + D);
            let p2 = ph + self.st_off;
            let q2 = p2 - round(p2) as f32;
            let mut t2 = (q2 * 0.5).abs();
            t2 = t2 + t2;
            t2 = t2 + t2;
            self.lfo2 += self.slew * ((t2 - self.lfo2) + D);
            let p = ph + self.inc;
            self.phase = p - round(p) as f32;
            m1 = self.lfo1;
            m2 = self.lfo2;
        } else {
            // Peak with hold, linear decay, and a 2 ms smoother.
            let lvl = if xl.abs() > xr.abs() { xl.abs() } else { xr.abs() };
            let mut pk = self.peak;
            if lvl > pk {
                self.hold_ctr = 0;
                pk = lvl;
                self.peak = pk;
            } else {
                let old = self.hold_ctr;
                self.hold_ctr = old + 1;
                if self.hold <= old {
                    pk = (pk - self.decay).max(0.0);
                    self.peak = pk;
                }
            }
            let target = (pk + c(0x3586_37bd)).min(1.0); // + 1e-6
            let e = self.env + self.env_c * (target - self.env);
            self.env = e + D;
            m1 = e;
            m2 = self.st_gain * e;
        }
        let hz_r = note_hz((self.freq + m2 * self.amt) * OCT);
        let hz_l = note_hz((self.freq + m1 * self.amt) * OCT);
        for (ch, hz) in [(0usize, hz_l), (1, hz_r)] {
            let hz = if hz > 14000.0 { 14000.0 } else { hz };
            let hz = if hz > self.hi_hz {
                self.hi_hz
            } else if self.lo_hz <= hz {
                hz
            } else {
                self.lo_hz
            };
            let (eight, fb, a, w_k) = (self.eight, self.fb, self.a, self.w_k);
            let chan = &mut self.chans[ch];
            chan.coefs(hz * w_k);
            let b0 = chan.b0;
            let b3 = b0 * (b0 * b0);
            let sel = if eight { b0 } else { 1.0 };
            chan.den = 1.0 - fb * (a * (sel * b3));
        }
    }

    /// `PhaserCore::processAudio` for one sample (§4).
    fn tick(&mut self, xl: f32, xr: f32) -> (f32, f32) {
        // The one-sample delays move first.
        let mut old = [[0f32; 5]; 2];
        for (o, ch) in old.iter_mut().zip(self.chans.iter_mut()) {
            *o = ch.d;
            for k in 0..5 {
                ch.d[k] = ch.n[k] + D;
            }
        }
        if self.counter < self.period {
            self.counter += 1;
        } else {
            self.counter = 0;
            self.control(xl, xr);
        }
        let (eight, a, aa, fb) = (self.eight, self.a, self.aa, self.fb);
        let (dry, wet, g, g_m1, g_inv) = (self.dry, self.wet, self.g, self.g_m1, self.g_inv);
        let (hp_g, hp_inv, hp_m1) = (self.hp_g, self.hp_inv, self.hp_m1);
        let mut out = [0f32; 2];
        for (ch, x) in [(0usize, xl), (1, xr)] {
            let c = &mut self.chans[ch];
            let od = old[ch];
            let mut s = [0f32; 4];
            for k in 0..4 {
                s[k] = c.n[k] * c.c1 + od[k] * c.c2 + c.n[k + 1] * c.c3 + od[k + 1] * c.c4;
            }
            let b0 = c.b0;
            let t3 = s[2] + b0 * (s[1] + b0 * (c.b0z + s[0]));
            let fbo = if eight { s[3] + b0 * t3 } else { t3 };
            // The loop node, solved implicitly.
            let u = (x + fb * (c.z * aa + a * fbo)) / c.den;
            // The high-passed part, added back scaled.
            let h = c.hp;
            let t = hp_inv * (u - h * hp_m1);
            c.hp = t + D;
            let v = u + HP_K * (hp_g * (h + t));
            // Output: dry input and loop node, through two one-pole low-passes.
            let (l1, l2) = (c.lp1, c.lp2);
            let v1 = g_inv * ((x * dry + u * wet) - l1 * g_m1);
            c.lp1 = v1 + D;
            let v2 = g_inv * (g * (l1 + v1) - l2 * g_m1);
            c.lp2 = v2 + D;
            out[ch] = g * (l2 + v2);
            // Clip and run the allpass chain.
            let cl = if v > CLIP {
                CLIP
            } else if -CLIP <= v {
                v
            } else {
                -CLIP
            };
            let x0 = cl * (1.0 - cl.abs() * CLIP_K);
            let y1 = s[0] + b0 * x0;
            let y2 = s[1] + b0 * y1;
            let y3 = s[2] + b0 * y2;
            let y4 = s[3] + b0 * y3;
            c.n = [x0 + D, y1 + D, y2 + D, y3 + D, y4 + D];
            let y = if eight { y4 } else { y3 };
            c.z = (y - c.z * a) + D;
        }
        (out[0], out[1])
    }
}

/// `8.17742·2^x` with the kernel's quadratic `2^frac`.
fn note_hz(x: f32) -> f32 {
    let n = round(x);
    let f = x - n as f32;
    (f * (f * c(0x3e7d_f3b6) + c(0x3f35_3f7d)) + 1.0) * pow2i(n) * NOTE0
}

/// Field names and defaults of the float parameters, in `tFloatParameters` order:
/// Frequency, Amount, Speed, Shape, Stereo, Feedback, Mix.
const PARAMS: [(&str, f32); 7] =
    [("frequency", 0.5), ("amount", 0.5), ("speed", 0.5), ("shape", 0.5), ("stereo", 0.5), ("feedback", 0.5), ("mix", 0.5)];

/// Maschine's processing block (samples): the LFO re-phase, rate and smoothing stop come
/// once per block (§3).
const BLOCK: usize = 128;

/// The Phaser effect.
pub struct Phaser {
    core: Core,
    lfo: Lfo,
    bpm: f32,
    /// The song position (quarters) at the next block, while the transport plays.
    quarters: Option<f64>,
    /// [`PARAMS`], then Source and 8 Pole.
    values: Option<[f32; 9]>,
}

impl Phaser {
    pub fn new(sr: f64) -> Self {
        Phaser { core: Core { sr: sr as f32, ..Core::default() }, lfo: Lfo::new(), bpm: 120.0, quarters: None, values: None }
    }

    /// `processFloatParameter` (`k` 0..6), `processEnumParameter` (7, Source) and
    /// `processBoolParameter` (8, 8 Pole).
    fn apply(&mut self, k: usize, v: f32) {
        let core = &mut self.core;
        match k {
            0 => core.set_freq(v),
            1 => core.set_amount(v),
            2 => {
                self.lfo.speed = v;
                self.lfo.update();
            }
            3 => core.set_shape(v),
            4 => core.set_stereo(v),
            5 => core.set_feedback(v),
            6 => core.set_mix(v),
            7 => {
                let s = v as i64;
                self.lfo.mode = match s {
                    0 => 1,
                    1 => 2,
                    _ => 0,
                };
                self.lfo.update();
                core.set_env(s == 2);
            }
            _ => core.set_eight(v != 0.0),
        }
    }

    /// `PhaserProcessor::process` for one of Maschine's blocks: the LFO's rate (and, synced
    /// with the transport playing, phase) for the block, the smoothing stop, then the core
    /// per sample.
    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.lfo.frame(self.bpm, self.quarters);
        let core = &mut self.core;
        core.set_lfo_rate(self.lfo.out);
        if let Some(p) = self.lfo.reset {
            core.phase = p;
        }
        let settled = [
            (core.freq_c as f32, core.freq_t),
            (core.amt_c as f32, core.amt_t),
            (core.fb_c as f32, core.fb_t),
            (core.dry_c as f32, core.dry_t),
            (core.wet_c as f32, core.wet_t),
            (core.lpf_c as f32, core.lpf_t),
        ];
        for (bit, (cur, t)) in settled.iter().enumerate() {
            if (*cur as f64 - t).abs() <= SETTLED {
                core.flags &= !(1 << bit);
            }
        }
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            (*a, *b) = core.tick(*a, *b);
        }
    }
}

impl Effect for Phaser {
    fn set(&mut self, params: &[(String, f64)]) {
        let mut new = [0f32; 9];
        for (k, (name, def)) in PARAMS.iter().enumerate() {
            new[k] = param(params, name, *def as f64) as f32;
        }
        new[7] = param(params, "source", 1.0) as f32;
        new[8] = param(params, "eight_pole", 0.0) as f32;
        match self.values {
            None => {
                // The parameters reach the processor (Source before Speed), then it
                // resumes at the host's rate.
                for k in [0, 1, 7, 2, 3, 4, 5, 8, 6] {
                    self.apply(k, new[k]);
                }
                self.core.resume();
            }
            Some(old) => {
                for k in 0..9 {
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

    /// The transport for the next block: a synced LFO follows it.
    fn set_transport(&mut self, quarters: Option<f64>) {
        self.quarters = quarters;
    }

    /// Maschine runs its effects in blocks of [`BLOCK`] samples (§3).
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (x, y) in l.chunks_mut(BLOCK).zip(r.chunks_mut(BLOCK)) {
            self.run(x, y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(kv: &[(&str, f64)]) -> Vec<(String, f64)> {
        kv.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn noise(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i * 7919 + 13) % 1000) as f32 / 1000.0 - 0.5).collect()
    }

    fn run(fx: &mut Phaser, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut a, mut b) = (l.to_vec(), r.to_vec());
        for (x, y) in a.chunks_mut(512).zip(b.chunks_mut(512)) {
            fx.process(x, y);
        }
        (a, b)
    }

    #[test]
    fn exp2_table_matches_maschines() {
        let t = exp2_table();
        assert_eq!(t[0].to_bits(), 0x3f35_04f7);
        assert_eq!(t[1021].to_bits(), 0x3fb5_04d5);
        assert!((exp2_lookup(-2.5) / (-2.5f32).exp2() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn lfo_rates() {
        let mut l = Lfo::new();
        l.speed = 0.0;
        assert_eq!(l.free_rate(), 1.0 / 32.0);
        l.speed = 1.0;
        assert_eq!(l.free_rate(), 8.0);
        l.speed = 0.5;
        assert_eq!(l.synced_beats(), 2.0); // 1/2 bar
    }

    #[test]
    fn defaults_phase_the_input() {
        let mut fx = Phaser::new(44_100.0);
        fx.set(&[]);
        fx.set_tempo(120.0);
        let x = noise(44_100);
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        let diff: f32 = l.iter().zip(&x).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 100.0, "{diff}");
        // Stereo offsets the right channel's sweep.
        assert!(l.iter().zip(&r).any(|(a, b)| a != b));
    }

    #[test]
    fn channels_are_independent() {
        let mut fx = Phaser::new(48_000.0);
        fx.set(&params(&[("feedback", 1.0), ("eight_pole", 1.0), ("source", 0.0)]));
        let x = noise(30_000);
        let z = vec![0.0; x.len()];
        let (l, r) = run(&mut fx, &x, &z);
        assert!(l.iter().any(|v| v.abs() > 1e-3));
        assert!(r.iter().all(|v| v.abs() < 1e-12));
    }

    #[test]
    fn loud_input_stays_bounded() {
        for src in [0.0, 1.0, 2.0] {
            let mut fx = Phaser::new(96_000.0);
            fx.set(&params(&[("feedback", 1.0), ("amount", -1.0), ("mix", 1.0), ("source", src), ("speed", 1.0)]));
            let x: Vec<f32> = noise(100_000).iter().map(|v| v * 40.0).collect();
            let (l, r) = run(&mut fx, &x, &x);
            assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 1e3));
        }
    }

    /// Null test of a synced LFO with the transport playing: `PHASER_PLAY=in.wav,out.wav`
    /// (`maschine-ref render --play --tempo 120`), the defaults with Speed 0.8.
    #[test]
    #[ignore]
    fn synced_with_transport() {
        let Ok(v) = std::env::var("PHASER_PLAY") else { return };
        let (inp, out) = v.split_once(',').unwrap();
        let mut rd = hound::WavReader::open(inp).unwrap();
        let sr = rd.spec().sample_rate as f64;
        let x: Vec<f32> = rd.samples::<f32>().map(Result::unwrap).collect();
        let (mut l, mut r): (Vec<f32>, Vec<f32>) = (x.iter().step_by(2).copied().collect(), x.iter().skip(1).step_by(2).copied().collect());
        let mut f = Phaser::new(sr);
        f.set(&params(&[("speed", 0.8)]));
        f.set_tempo(120.0);
        let blk: usize = std::env::var("PHASER_BLOCK").map_or(1024, |b| b.parse().unwrap());
        let off: f64 = std::env::var("PHASER_OFFSET").map_or(0.0, |b| b.parse().unwrap());
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

    #[test]
    fn set_keeps_state_and_smooths() {
        let x = noise(40_960);
        let p = params(&[("frequency", 0.3), ("feedback", 0.7)]);
        let mut a = Phaser::new(44_100.0);
        a.set(&p);
        let (l1, _) = run(&mut a, &x[..10_240], &x[..10_240]);
        a.set(&p);
        let (l2, _) = run(&mut a, &x[10_240..], &x[10_240..]);
        let mut b = Phaser::new(44_100.0);
        b.set(&p);
        let (lb, _) = run(&mut b, &x, &x);
        assert_eq!([l1, l2].concat(), lb);

        // A Frequency change glides.
        a.set(&params(&[("frequency", 0.9), ("feedback", 0.7)]));
        run(&mut a, &x[..64], &x[..64]);
        let (f, t) = (a.core.freq, a.core.freq_t as f32);
        assert!(f < t - 1.0, "{f} {t}");
        run(&mut a, &x, &x);
        assert!((a.core.freq - t).abs() < 1e-2);
    }
}
