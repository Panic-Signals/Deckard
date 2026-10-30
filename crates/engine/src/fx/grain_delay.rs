//! Maschine's Grain Delay (docs/fx-grain-delay.md): `RT::GrainDelayProcessor` →
//! `FXCORE::GrainDelayCore`, a compiled Reaktor-Core kernel. Two grain voices (A reads the
//! left delay line and feeds the left output, B the right) are retriggered alternately by a
//! control-rate grain clock. A grain is a Hann-windowed read at `ratio = 2^(pitch/12)`
//! that starts `(max(ratio, 1) − 1)·len` behind the write head; every read is the mean of
//! four linear interpolations at sub-positions between this and the last three positions.
//! "Space" is a feedback cloud: two short delay lines, read with a triangle-LFO-modulated
//! delay (Mod), cross-coupled through one-pole LP/HP filters, a 2× interpolation and a soft
//! clipper. There is no random number generator: Jitter alternates the grain period
//! between `size + j` and `size − j`.
//!
//! All arithmetic follows the assembly operation by operation (`f32`, `f64` where the kernel
//! uses doubles, no fused multiply-adds), so the output can match bit for bit.

use super::{Effect, param};

/// The two grain delay lines (`& 0x3ffff`) and the two space lines (`& 0x1ffff`).
const LINE: usize = 1 << 18;
const LINE_MASK: u32 = (LINE - 1) as u32;
const SPACE: usize = 1 << 17;
const SPACE_MASK: u32 = (SPACE - 1) as u32;
/// Offsets that keep the smoothers and filters off denormals.
const TINY: f32 = f32::from_bits(0x2193_92ef); // 1e-18
const TINY_D: f64 = f64::from_bits(0x2b2b_ff2e_e48e_0530); // 1e-100
/// A voice is retriggered only when its window is below this (`0x3c23d892`).
const RESTART: f32 = f32::from_bits(0x3c23_d892); // 0.010000365
/// The longest grain read, in samples (`0x48800000`).
const MAX_LEN: f32 = 262_144.0;

/// `f32` from its bits (the kernel's constants).
const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}

// Sub-position weights of the four-point read (cubic Lagrange at 1/4, 1/2, 3/4).
const W273: f32 = c(0x3e8c_0011); // 0.273438
const W820: f32 = c(0x3f52_0008); // 0.820313
const W054: f32 = c(0x3d60_0000); // 0.0546875
const W039: f32 = c(0x3d20_0000); // 0.0390625

/// Semitones → octaves (`0x3daaa880` = 0.0833292) and the frequency of note 0
/// (`0x4102d6b6` = 8.17742 Hz).
const DIV12: f32 = c(0x3daa_a880);
const NOTE0: f32 = c(0x4102_d6b6);

/// The space clipper's gain against Density's upper half, 0 … 127
/// (`GrainDelayCore::s_0694FDF8`, 0x10dc30c7c), as stored.
const SAT_GAIN: [f32; 128] = [
    1.0, 0.986452, 0.953786, 0.910191, 0.862307, 0.81444, 0.769763, 0.729232, 0.690348, 0.653279, 0.618146, 0.585151, 0.554405, 0.525945,
    0.499753, 0.475766, 0.453892, 0.434019, 0.415965, 0.39968, 0.385046, 0.371223, 0.358072, 0.345646, 0.333903, 0.322771, 0.312233,
    0.302249, 0.292817, 0.28388, 0.275453, 0.267466, 0.259946, 0.252818, 0.246109, 0.239757, 0.233769, 0.228118, 0.222765, 0.217747,
    0.212986, 0.208504, 0.204285, 0.200284, 0.196534, 0.193002, 0.189654, 0.186508, 0.183427, 0.180423, 0.177503, 0.174727, 0.172028,
    0.169395, 0.16687, 0.164426, 0.162043, 0.159723, 0.157515, 0.15536, 0.153257, 0.151227, 0.149282, 0.147383, 0.145529, 0.143747,
    0.142036, 0.140365, 0.138734, 0.137162, 0.135659, 0.134191, 0.132757, 0.131362, 0.130044, 0.128757, 0.127499, 0.126269, 0.125096,
    0.12397, 0.12287, 0.121794, 0.120743, 0.11975, 0.11879, 0.117852, 0.116936, 0.116041, 0.115188, 0.114374, 0.113579, 0.112802, 0.112043,
    0.111302, 0.110611, 0.10994, 0.109286, 0.108647, 0.108023, 0.107415, 0.106841, 0.106285, 0.105728, 0.105177, 0.104631, 0.10409,
    0.103555, 0.103054, 0.102561, 0.102074, 0.10159, 0.101112, 0.100638, 0.100169, 0.0997342, 0.0993044, 0.0988787, 0.0984569, 0.098039,
    0.0976251, 0.097215, 0.0968326, 0.0964594, 0.0960897, 0.0957232, 0.09536, 0.0950002, 0.0946436,
];

/// `v` rounded to 6 significant digits, as the kernel's tables are printed.
fn six_digits(v: f64) -> f32 {
    if v == 0.0 {
        return 0.0;
    }
    let p = 10f64.powi(5 - v.abs().log10().floor() as i32);
    ((v * p).round() / p) as f32
}

/// The octave table `2^(i/1021 − 0.5)`, i = 0 … 1021 (`s_04C30350`, the same as
/// Chorus's), with seven near-ties stored the other way; then √2 and two entries 1/√2
/// (read when the index rounds to −1, at an exact power of two).
fn exp_table() -> &'static [f32; 1025] {
    static T: std::sync::OnceLock<[f32; 1025]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0f32; 1025];
        for (i, e) in t.iter_mut().enumerate().take(1022) {
            *e = six_digits((i as f64 / 1021.0 - 0.5).exp2());
        }
        let ties = [(106, 0.759867), (213, 0.817119), (225, 0.823803), (287, 0.859218), (324, 0.881074), (382, 0.91646), (673, 1.11663)];
        for (i, v) in ties {
            t[i] = v;
        }
        t[1022] = c(0x3fb5_04d5); // 1.41421
        t[1023] = c(0x3f35_04f7); // 0.707107
        t[1024] = c(0x3f35_04f7);
        t
    })
}

/// The mantissa table `log2(1 + i/1021)`, i = 0 … 1021 (`s_04C31388`), with nine
/// near-ties stored the other way; then 1 and two zeros (read when the index rounds to
/// −1, at an exact power of two).
fn log_table() -> &'static [f32; 1025] {
    static T: std::sync::OnceLock<[f32; 1025]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0f32; 1025];
        for (i, e) in t.iter_mut().enumerate().take(1022) {
            *e = six_digits((1.0 + i as f64 / 1021.0).log2());
        }
        let ties = [
            (27, 0.0376558),
            (88, 0.119277),
            (264, 0.331786),
            (297, 0.368367),
            (377, 0.453382),
            (435, 0.512028),
            (439, 0.515985),
            (560, 0.630854),
            (935, 0.937923),
        ];
        for (i, v) in ties {
            t[i] = v;
        }
        t[1022] = 1.0;
        t
    })
}

/// `round()` as the kernel does it: add ±0.5 (by the sign) in `f64`, truncate.
fn rnd(x: f32) -> i32 {
    let h = if x < 0.0 { -0.5 } else { 0.5 };
    (h + x as f64) as i32
}

/// `2^n` for an integer `n`, made by adding `n` to the exponent bits of 1.0.
fn pow2i(n: i32) -> f32 {
    f32::from_bits(0x3f80_0000u32.wrapping_add((n as u32) << 23))
}

/// `2^x` from the octave table, linearly interpolated.
fn exp2_approx(x: f32) -> f32 {
    let t = exp_table();
    let n = rnd(x);
    let y = ((x - n as f32) + 0.5) * 1021.0;
    let k = rnd(y + -0.5);
    let fr = y - k as f32;
    let i = (k as u32 & 0x3ff) as usize;
    (t[i + 1] * fr + t[i] * (1.0 - fr)) * pow2i(n)
}

/// `log2(x)` for `x > 0` from the mantissa table, linearly interpolated, plus the exponent.
fn log2_approx(x: f32) -> f32 {
    let t = log_table();
    let b = x.to_bits();
    let m = f32::from_bits(b & 0x807f_ffff | 0x3f80_0000);
    let e = ((b >> 23) & 0xff) as i32 - 127;
    let y = (m + -1.0) * 1021.0;
    let k = rnd(y + -0.5);
    let fr = y - k as f32;
    let i = (k as u32 & 0x3ff) as usize;
    ((1.0 - fr) * t[i] + fr * t[i + 1]) + e as f32
}

/// `x^(1/3)` as the kernel computes it: `2^(log2(x)·0.63093·0.333333·1.58496)`.
fn cbrt_approx(x: f32) -> f32 {
    exp2_approx(log2_approx(x) * c(0x3f21_84a1) * c(0x3eaa_aa9f) * c(0x3fca_dff8))
}

/// Note → Hz with the kernel's cubic `2^x`: `8.17742·2^(note/12)`.
fn note_hz(note: f32) -> f32 {
    let x = note * DIV12;
    let n = rnd(x);
    let f = x - n as f32;
    let p = f * (f * (f * c(0x3d65_6042) + c(0x3e77_ced9)) + c(0x3f31_6873));
    (p + 1.0) * pow2i(n) * NOTE0
}

/// `0.1·2^(ms/20 − 1)` (quadratic `2^x`): with the Size in ms it sets how much Space
/// can stretch the space delay (limited to 5 by the callers).
fn space_factor(ms: f32) -> f32 {
    let x = ms * c(0x3d4c_cccd) + -1.0;
    let n = rnd(x);
    let f = x - n as f32;
    (f * (f * c(0x3e7d_f3b6) + c(0x3f35_3f7d)) + 1.0) * pow2i(n) * c(0x3dcc_cccd)
}

/// The kernel's `tan` approximation `u = t + t³/3 + 2t⁵/15` as `(u/(1+u), (1−u)/(1+u))`,
/// plus `1/(1+u)`.
fn tan_coefs(t: f32) -> (f32, f32, f32) {
    let t2 = t * t;
    let t3 = t * t2;
    let u = t + (t3 * c(0x3eaa_aa9f) + (t2 * t3) * c(0x3e08_8872));
    let d = u + 1.0;
    (u / d, (1.0 - u) / d, 1.0 / d)
}

/// `(1 + cos(2π·p − π))/2` with the kernel's polynomial: the grain window at phase `p`.
fn window(p: f32) -> f32 {
    let x = p * c(0x40c9_0fd0) + c(0xc049_0fd0);
    let x2 = x * x;
    let mut s = c(0x37cf_b5a1) - x2 * c(0x348b_dad9);
    s = x2 * s + c(0xbab6_09bd);
    s = x2 * s + c(0x3d2a_aa99);
    s = x2 * s + c(0xbeff_f5e8);
    (x2 * s + 2.0) * 0.5
}

/// `x` limited to ±8 and soft-clipped to `x·(1 − |x|/16)`.
fn soft_clip(x: f32) -> f32 {
    let x = if x < -8.0 { -8.0 } else { x };
    let x = if x < 8.0 { x } else { 8.0 };
    x * (1.0 - x.abs() * 0.0625)
}

/// `0 … 1` limit with the kernel's comparisons.
fn unit(x: f32) -> f32 {
    let y = if x < 0.0 { 0.0 } else { x };
    if x < 1.0 { y } else { 1.0 }
}

/// Index and fraction into `SAT_GAIN` for Density's upper half.
fn sat_index(v: f32) -> (i32, f32) {
    let x = v * 127.0;
    let i = rnd(x + -0.5);
    (i, x - i as f32)
}

/// `SAT_GAIN` interpolated with the kernel's index limits.
fn sat_lerp(i: i32, f: f32) -> f32 {
    let i0 = if (i as u32) < 127 { i as usize } else { 127 };
    let i1 = if (i.wrapping_add(1) as u32) >= 127 { 127 } else { (i + 1) as usize };
    let g0 = SAT_GAIN[i0];
    g0 + (SAT_GAIN[i1] - g0) * f
}

/// A one-pole parameter smoother in `f64` (target, value, coefficient) and its value
/// rounded to `f32` (kept as `f64`; the processor compares it with the target).
#[derive(Default, Clone, Copy)]
struct Smoother {
    t: f64,
    s: f64,
    c: f64,
    f: f64,
}

impl Smoother {
    fn step(&mut self) -> f32 {
        self.s += self.c * ((self.t - self.s) + TINY_D);
        let v = self.s as f32;
        self.f = v as f64;
        v
    }

    /// Whether the processor keeps it running (`|f − t| > 1e-4`).
    fn moving(&self) -> bool {
        (self.f - self.t).abs() > 1e-4
    }
}

/// One grain voice (A: +0x94 …, B: +0x1d0 …).
#[derive(Default, Clone, Copy)]
struct Voice {
    /// Window value (+0xa0 / +0x1d4).
    win: f32,
    /// `1/sr` (+0xac / +0x1d8); window-phase increment and phase (+0xb0 / +0xb4,
    /// +0x1dc / +0x1e0).
    inv_sr: f32,
    inc: f32,
    ph: f32,
    /// Reverse as the processor last sent it (+0xe0 / +0x1e4) and the read span over one
    /// window (+0xe4 / +0x1e8), taken at the grain's start.
    rev: i32,
    rate: f32,
    /// Start position (+0xf4 / +0x1ec) and the last three read positions.
    start: f32,
    h: [f32; 3],
}

impl Voice {
    /// The windowed four-point read at window phase `p` (0 … 1), shifting the positions.
    fn read(&mut self, buf: &[f32], p: f32) -> f32 {
        let pos = p * self.rate + self.start;
        let [h0, h1, h2] = self.h;
        let lerp = |q: f32| {
            let i = rnd(q + -0.5);
            let a = buf[(i.wrapping_sub(1) as u32 & LINE_MASK) as usize];
            let b = buf[(i as u32 & LINE_MASK) as usize];
            a + (b - a) * ((i as f32 - q) + 1.0)
        };
        let l1 = lerp((h0 * W273 + (h1 * W820 - h2 * W054)) - pos * W039);
        let l2 = lerp((h0 * 0.5625 + (h1 * 0.5625 - h2 * 0.0625)) - pos * 0.0625);
        let l3 = lerp((h0 * W820 + (h1 * W273 - h2 * W039)) - pos * W054);
        let l4 = lerp(h0);
        self.h = [pos, h0, h1];
        self.win = window(p);
        self.win * ((l4 + (l3 + (l1 + l2))) * 0.25)
    }
}

/// One side of the space feedback: LP (+0x314 …) and HP (+0x2f0 …) one-poles, the 2×
/// interpolation history (+0x32c …) and the value fed back (+0x150; the other side's
/// fields are +0x368 …, +0x340 …, +0x378 …, +0x200).
#[derive(Default, Clone, Copy)]
struct Side {
    /// LP: last input + tiny, last output + tiny; `a = u/(1+u)`, `b = (1−u)/(1+u)`.
    x1: f32,
    y1: f32,
    lp_a: f32,
    lp_b: f32,
    /// HP: last output + tiny; `g = 1/(1+u)`, `b = (1−u)/(1+u)`.
    h1: f32,
    hp_g: f32,
    hp_b: f32,
    /// Interpolation history, newest first.
    u: [f32; 3],
    /// The value written into a space line on the next sample.
    fb: f32,
}

impl Side {
    fn set_hp(&mut self, t: f32) {
        let (_, b, g) = tan_coefs(t);
        (self.hp_b, self.hp_g) = (b, g);
    }

    fn set_lp(&mut self, t: f32) {
        let (a, b, _) = tan_coefs(t);
        (self.lp_a, self.lp_b) = (a, b);
    }

    /// Filter, interpolate and clip `x` into the next feedback value.
    fn feed(&mut self, x: f32, up: f32, sat: f32, wet: f32) {
        let (x1, y1) = (self.x1, self.y1);
        self.x1 = x + TINY;
        let y = x1 * self.lp_a + (y1 * self.lp_b + x * self.lp_a);
        self.y1 = y + TINY;
        let hp = y1 * -self.hp_g + (self.h1 * self.hp_b + y * self.hp_g);
        self.h1 = hp + TINY;
        let n = up * hp;
        let [u0, u1, u2] = self.u;
        let mid = (u0 * 0.5625 + (u1 * 0.5625 - u2 * 0.0625)) - n * 0.0625;
        self.u = [n, u0, u1];
        let s = (soft_clip(u0) + soft_clip(mid)) * 0.5 * sat;
        self.fb = hp + wet * (s - hp) + TINY;
    }
}

/// `FXCORE::GrainDelayCore` (its fields with their object offsets; §3).
struct Core {
    sr: f32,
    // Inputs as the processor sends them (stored values; Pitch in semitones).
    mix_in: f32,    // +0x10
    size_in: f32,   // +0x18
    rev_in: f32,    // +0x20
    pitch_in: f32,  // +0x28
    mod_in: f32,    // +0x30
    space_in: f32,  // +0x38
    dens_in: f32,   // +0x40
    stereo_in: f32, // +0x4c
    jit_in: f32,    // +0x64
    /// Control counter (+0x6c), its last value (+0x70) and the smoothing flags (+0x74).
    ctl: i32,
    ctl_last: i32,
    flags: u32,
    // Smoothers: Mix +0x78, Pitch +0xb8, Mod +0x120, feedback +0x180, Density's upper
    // half +0x1b0, Stereo +0x208, space LP note +0x270.
    mix: Smoother,
    pitch: Smoother,
    modu: Smoother,
    fbk: Smoother,
    dens_hi: Smoother,
    stereo: Smoother,
    lp: Smoother,
    /// Dry (+0x90), wet (+0x224) and Stereo (+0x220) gains.
    dry: f32,
    wet: f32,
    width: f32,
    /// Grain clock: control steps since the last grain (+0x98), the voice toggle (+0x9c),
    /// the toggle at the last period computation (+0x2b8) and the period (+0x2ec).
    grain_cnt: i32,
    tog: i32,
    tog_seen: i32,
    period: i32,
    /// Size in ms (+0xa4; `f64` +0x2c8) and `1000/ms` (+0xa8).
    size_ms: f32,
    size_d: f64,
    grain_hz: f32,
    a: Voice,
    b: Voice,
    /// `1/(8.17742·2^0)` with the table's 2^0 (+0xd0); the pitch ratio (+0xd4) and
    /// `max(ratio, 1)` (+0xec); samples per ms (+0xd8 = +0x13c = +0x164); Size in samples
    /// (+0xdc); the start offset (+0xf0); read spans forward / reverse (+0x384 / +0x388).
    ratio_k: f32,
    ratio: f32,
    ratio1: f32,
    sr_ms: f32,
    glen: f32,
    offset: f32,
    spans: [f32; 2],
    /// Write positions of the grain lines (+0xe8) and the space lines (+0x104).
    w: u32,
    ws: u32,
    /// Triangle LFO for Mod: 0.03125537 (+0x108), `1/sr` (+0x10c), increment (+0x110),
    /// phase in [−0.5, 0.5) (+0x114).
    lfo_k: f32,
    lfo_isr: f32,
    lfo_inc: f32,
    lfo: f32,
    /// Mod: target `a + Mod²·(0.1 − a)` (+0x118 / +0x11c); smoothed − 1 (+0x138 = +0x1fc).
    mod_a: f32,
    mod_b: f32,
    mod_depth: f32,
    /// Space delay in samples: target (+0x140), value (+0x144), coefficient (+0x148),
    /// current (+0x14c).
    dly_t: f32,
    dly_s: f32,
    dly_c: f32,
    dly: f32,
    /// `log2(…) − 16` (+0x154); base (+0x158), Space (+0x15c) and cube root (+0x160) of
    /// the space amount; its cube (+0x2d0).
    k154: f32,
    sp_base: f32,
    space: f32,
    sp_root: f32,
    cube: f64,
    /// Space level per feedback (+0x168).
    sp_gain: f64,
    /// Feedback: low end (+0x170), Density amount (+0x174), range (+0x178), value (+0x198).
    fb_lo: f32,
    dens_x: f32,
    fb_rng: f32,
    fb: f32,
    /// Space level smoother: target (+0x19c), value (+0x1a4), coefficient (+0x1a8),
    /// current (+0x1ac).
    lvl_t: f32,
    lvl_s: f32,
    lvl_c: f32,
    lvl: f32,
    /// Width scale −0.20555687 (+0x1c8) and the width gain (+0x1cc).
    wid_k: f32,
    wid: f32,
    /// Space LP: amount (+0x268) and note range (+0x26c); limits `sr/24576`, `sr/2.125`
    /// (+0x2fc …, +0x34c …) and `π/sr` (+0x308 = +0x35c).
    lp_amt: f32,
    lp_k: f32,
    lo: f32,
    hi: f32,
    pi_sr: f32,
    /// Jitter² (+0x2c0), its scale (+0x2d8), the ±values (+0x300390) and the one in use
    /// (+0x2e0).
    jit2: f64,
    jit_scale: f64,
    jit: [f64; 2],
    jit_cur: f64,
    /// Control steps per ms (+0x2e8).
    per_ms: f32,
    /// 2× interpolation gain scale 18.953209 (+0x324) and gain (+0x328), clipper gain
    /// (+0x338), the space feedback's wet amount (+0x33c).
    up_k: f32,
    up: f32,
    sat: f32,
    sp_wet: f32,
    /// Fed by voice B, written into `sp_l`; fed by voice A, written into `sp_r`.
    left: Side,
    right: Side,
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    sp_l: Vec<f32>,
    sp_r: Vec<f32>,
}

impl Core {
    /// All zero (as `resume` leaves it) with the default inputs and no buffers.
    fn blank(sr: f32) -> Core {
        Core {
            sr,
            mix_in: 0.75,
            size_in: 0.25,
            rev_in: 0.0,
            pitch_in: 0.0,
            mod_in: 0.0,
            space_in: 0.0,
            dens_in: 0.25,
            stereo_in: 0.75,
            jit_in: 0.0,
            ctl: 0,
            ctl_last: 0,
            flags: 0,
            mix: Smoother::default(),
            pitch: Smoother::default(),
            modu: Smoother::default(),
            fbk: Smoother::default(),
            dens_hi: Smoother::default(),
            stereo: Smoother::default(),
            lp: Smoother::default(),
            dry: 0.0,
            wet: 0.0,
            width: 0.0,
            grain_cnt: 0,
            tog: 0,
            tog_seen: 0,
            period: 0,
            size_ms: 0.0,
            size_d: 0.0,
            grain_hz: 0.0,
            a: Voice::default(),
            b: Voice::default(),
            ratio_k: 0.0,
            ratio: 0.0,
            ratio1: 0.0,
            sr_ms: 0.0,
            glen: 0.0,
            offset: 0.0,
            spans: [0.0; 2],
            w: 0,
            ws: 0,
            lfo_k: 0.0,
            lfo_isr: 0.0,
            lfo_inc: 0.0,
            lfo: 0.0,
            mod_a: 0.0,
            mod_b: 0.0,
            mod_depth: 0.0,
            dly_t: 0.0,
            dly_s: 0.0,
            dly_c: 0.0,
            dly: 0.0,
            k154: 0.0,
            sp_base: 0.0,
            space: 0.0,
            sp_root: 0.0,
            cube: 0.0,
            sp_gain: 0.0,
            fb_lo: 0.0,
            dens_x: 0.0,
            fb_rng: 0.0,
            fb: 0.0,
            lvl_t: 0.0,
            lvl_s: 0.0,
            lvl_c: 0.0,
            lvl: 0.0,
            wid_k: 0.0,
            wid: 0.0,
            lp_amt: 0.0,
            lp_k: 0.0,
            lo: 0.0,
            hi: 0.0,
            pi_sr: 0.0,
            jit2: 0.0,
            jit_scale: 0.0,
            jit: [0.0; 2],
            jit_cur: 0.0,
            per_ms: 0.0,
            up_k: 0.0,
            up: 0.0,
            sat: 0.0,
            sp_wet: 0.0,
            left: Side::default(),
            right: Side::default(),
            buf_l: Vec::new(),
            buf_r: Vec::new(),
            sp_l: Vec::new(),
            sp_r: Vec::new(),
        }
    }

    fn new(sr: f32) -> Core {
        Core { buf_l: vec![0.0; LINE], buf_r: vec![0.0; LINE], sp_l: vec![0.0; SPACE], sp_r: vec![0.0; SPACE], ..Core::blank(sr) }
    }

    /// `GrainDelayProcessor::resume`: zero the kernel (keeping the inputs), then `endInit`.
    fn resume(&mut self) {
        let mut z = Core::blank(self.sr);
        (z.mix_in, z.size_in, z.rev_in) = (self.mix_in, self.size_in, self.rev_in);
        (z.pitch_in, z.mod_in, z.space_in) = (self.pitch_in, self.mod_in, self.space_in);
        (z.dens_in, z.stereo_in, z.jit_in) = (self.dens_in, self.stereo_in, self.jit_in);
        z.buf_l = std::mem::take(&mut self.buf_l);
        z.buf_r = std::mem::take(&mut self.buf_r);
        z.sp_l = std::mem::take(&mut self.sp_l);
        z.sp_r = std::mem::take(&mut self.sp_r);
        for b in [&mut z.buf_l, &mut z.buf_r, &mut z.sp_l, &mut z.sp_r] {
            b.fill(0.0);
        }
        *self = z;
        self.end_init();
    }

    /// A filter frequency limited to `[sr/24576, sr/2.125]` (the kernel's comparisons).
    fn band(&self, f: f32) -> f32 {
        if f > self.hi {
            self.hi
        } else if f < self.lo {
            self.lo
        } else {
            f
        }
    }

    /// `endInit` (0x101a364cc), as `resume` runs it (every input flagged): coefficients and
    /// targets, smoothers started at their targets, one control step that starts voice B,
    /// and one sample on silence (§3.2).
    fn end_init(&mut self) {
        let lt = log_table();
        let et = exp_table();
        let mod2 = self.mod_in * self.mod_in;
        let dens = self.dens_in;
        let size2 = self.size_in * self.size_in;
        self.wid_k = c(0xbe52_7d80);
        self.k154 = (lt[39] * c(0x3ef9_7180) + lt[40] * c(0x3f03_4740)) + -16.0;
        let sr = self.sr;
        let sr_ms = sr / 1000.0;
        self.pi_sr = c(0x4049_0fd0) / sr;
        self.lo = sr / 24576.0;
        self.hi = sr / 2.125;
        let (lo, hi) = (self.lo, self.hi);
        self.up_k = c(0x4197_a02c);
        // Control rate: a step every round(sr/8820) samples; steps per ms from the rate.
        let srr = rnd(sr);
        let steps = rnd(sr / 8820.0);
        let ctl_hz = if steps == 0 { 0 } else { srr.wrapping_div(steps) } as f32;
        let per_ms = ctl_hz / 1000.0;
        let per_ms_d = ctl_hz as f64 / 1000.0;
        self.ctl_last = steps - 1;
        self.flags = 1;
        self.per_ms = per_ms;
        self.b.inv_sr = 1.0 / sr;
        self.a.inv_sr = 1.0 / sr;
        let e0 = et[510] * 0.5 + et[511] * 0.5;
        self.ratio_k = 1.0 / (e0 * NOTE0);
        self.mod_a = -1.0 / (e0 * 65536.0) + 1.0;
        self.mod_b = c(0x3dcc_cccd) - self.mod_a;
        self.modu.t = (self.mod_a + mod2 * self.mod_b) as f64;
        let ms = size2 * 928.0 + 72.0;
        self.size_ms = ms;
        let p = space_factor(ms);
        self.size_d = ms as f64;
        let inv = 1000.0 / ms;
        self.lfo_k = c(0x3d00_05a1);
        self.lfo_isr = 1.0 / sr;
        self.lfo_inc = self.lfo_isr * (inv * self.lfo_k);
        let glen = sr_ms * ms;
        self.dly_t = glen;
        self.grain_hz = inv;
        self.sr_ms = sr_ms;
        self.glen = glen;
        self.mix.t = self.mix_in as f64;
        // log2(0.01) and log2(2^-61·…) from the log table, for the smoother coefficients.
        let l7 = (lt[285] * c(0x3df5_d000) + lt[286] * c(0x3f61_4600)) + -7.0;
        let l61 = (lt[245] * c(0x3f75_d200) + lt[246] * c(0x3d22_e000)) + -61.0;
        let d2 = dens + dens;
        let fl = if hi < 55.0 { hi } else { lo.max(55.0) };
        let fr = if hi < 45.0 { hi } else { lo.max(45.0) };
        self.space = self.space_in;
        let n250 = (250.0 * per_ms + -1.0).max(1.0);
        let n50 = (50.0 * per_ms + -1.0).max(1.0);
        let n150 = (150.0 * per_ms_d + -1.0).max(1.0);
        let n200 = (200.0 * per_ms_d + -1.0).max(1.0);
        let d2c = d2.min(1.0);
        let x = ms * p.min(5.0);
        let root = if x > 0.0 { cbrt_approx(x) } else { 0.0 };
        self.left.set_hp(self.pi_sr * fl);
        self.right.set_hp(self.pi_sr * fr);
        self.lvl_c = 1.0 - exp2_approx(l7 / n50);
        self.dly_c = 1.0 - exp2_approx(l7 / n250);
        let c200 = 1.0 - exp2_approx(((l61 as f64) / n200) as f32) as f64;
        let c150 = 1.0 - exp2_approx(((l61 as f64) / n150) as f32) as f64;
        self.mix.c = c200;
        self.stereo.c = c200;
        for s in [&mut self.pitch, &mut self.fbk, &mut self.modu, &mut self.dens_hi, &mut self.lp] {
            s.c = c150;
        }
        // The sample's input (zero after resume) goes into the grain lines.
        let w_old = self.w;
        self.w = (w_old + 1) & LINE_MASK;
        self.buf_r[w_old as usize] = 0.0;
        self.buf_l[w_old as usize] = 0.0;
        self.modu.s = self.modu.t;
        let dh2 = (d2c + -0.5) + (d2c + -0.5);
        let dx = unit(d2c + d2c);
        self.dens_hi.t = (d2 - d2c) as f64;
        self.dens_hi.s = self.dens_hi.t;
        self.sp_base = 0.0;
        self.sp_root = root;
        let sp = root * self.space + 0.0;
        let cube = sp * (sp * sp);
        let c8 = cube * c(0x3c03_126f);
        self.cube = cube as f64;
        self.dly_s = glen;
        self.jit2 = (self.jit_in * self.jit_in) as f64;
        let lp_amt = unit(cube * c(0x3b83_126f));
        let jscale = if ms > cube { ms as f64 } else { cube as f64 };
        let k = dh2 * 18.0 + 104.0 + -84.0;
        let k = if dh2 < 0.0 { 20.0 } else { k };
        let k = if dh2 < 1.0 { k } else { 38.0 };
        self.dens_x = dx;
        self.lp_amt = lp_amt;
        let fb_lo = if c8 < 0.0 { 0.0 } else { c8 * c(0x3f19_999a) + 0.0 };
        let fb_lo = if c8 < 1.0 { fb_lo } else { c(0x3f19_999a) };
        self.fb_lo = fb_lo;
        self.fb_rng = c(0x3f73_3333) - fb_lo;
        self.fbk.t = (fb_lo + dx * self.fb_rng) as f64;
        self.fbk.s = self.fbk.t;
        let d19 = ((cube.max(0.25) * sr_ms) as f64 + -1.0).max(1.0);
        self.jit_scale = jscale;
        self.jit = [-(jscale * self.jit2), jscale * self.jit2];
        let ph = self.lfo_inc + -0.5;
        self.lfo = ph - rnd(ph) as f32;
        self.lp_k = k * unit((dx + -0.75) * 4.0) + 84.0 + -96.0;
        self.lp.t = note_hz(self.lp_k * lp_amt + 96.0) as f64;
        self.lp.s = self.lp.t;
        let e = exp2_approx(((self.k154 as f64) / d19) as f32);
        self.sp_gain = e as f64;

        // The control step (the counter starts at its end).
        self.grain_cnt = 1;
        // `(glen − glen) + 1e-18` in the kernel: the smoother starting at its target.
        let dly = glen + TINY * self.dly_c;
        self.dly = dly;
        self.dly_s = dly;
        let modf = self.modu.step();
        let dhf = self.dens_hi.step();
        self.sp_wet = (2.0 - dhf) * dhf;
        let lpf = self.lp.step();
        let fbf = self.fbk.step();
        self.fb = fbf;
        self.mix.s = self.mix.t;
        let mixf = self.mix.step();
        self.wet = (2.0 - mixf) * mixf;
        self.pitch.t = self.pitch_in as f64;
        self.pitch.s = self.pitch.t;
        let ex = exp2_approx(self.pitch.step() * DIV12);
        self.stereo.t = self.stereo_in as f64;
        self.stereo.s = self.stereo.t;
        self.width = self.stereo.step();
        self.ctl = 0;
        self.tog = 0;
        self.up = dhf * self.up_k + 1.0;
        self.wid = dhf * self.wid_k + 1.0;
        self.right.set_lp(self.band(lpf) * self.pi_sr);
        self.lvl_t = e * fbf;
        self.lvl_s = self.lvl_t;
        self.mod_depth = modf + -1.0;
        let j = (self.jit_cur + self.cube).max(0.0) as f32;
        self.period = rnd(per_ms * ((ms + j) * 0.5));
        let (sat_i, sat_f) = sat_index(dhf);
        let start_b = RESTART > self.b.win;
        if start_b {
            self.b.ph = 0.0;
            self.b.inc = self.grain_hz * self.b.inv_sr;
        }
        let lvl = self.lvl_t + TINY * self.lvl_c;
        self.lvl = lvl;
        self.lvl_s = lvl;
        if self.period <= 1 {
            self.grain_cnt = 0;
        }
        self.left.set_lp(self.band(lpf + 10.0) * self.pi_sr);
        self.ratio = ex * NOTE0 * self.ratio_k;
        self.flags = 0x7f;
        let r = self.ratio;
        self.ratio1 = r.max(1.0);
        let lr = r * self.glen;
        let len = self.glen * (lr.min(MAX_LEN) / lr);
        self.offset = -(self.ratio1 * len - len);
        self.spans = [r * len, -(r * len)];
        if start_b {
            self.b.start = self.offset + w_old as i32 as f32;
        }
        self.rev_event();
        if start_b {
            self.b.rate = self.spans[self.b.rev as usize];
        }
        self.sat = sat_lerp(sat_i, sat_f);
        self.dry = (1.0 - mixf) * (2.0 - (1.0 - mixf));
        self.tail(0.0, 0.0, lvl);
    }

    /// `processBoolParameter` (Reverse): each voice takes it at its next start.
    fn rev_event(&mut self) {
        let h = if self.rev_in < 0.0 { -0.5 } else { 0.5 };
        let r = ((h + self.rev_in as f64) as i32 != 0) as i32;
        self.a.rev = r;
        self.b.rev = r;
    }

    /// `processEventInms` (Size).
    fn size_event(&mut self) {
        let ms = self.size_in * self.size_in * 928.0 + 72.0;
        self.size_d = ms as f64;
        let inv = 1000.0 / ms;
        self.size_ms = ms;
        self.grain_hz = inv;
        let p = space_factor(ms);
        self.lfo_inc = inv * self.lfo_k * self.lfo_isr;
        self.dly_t = ms * self.sr_ms;
        self.glen = ms * self.sr_ms;
        let r = self.ratio;
        let lr = self.glen * r;
        let x = ms * p.min(5.0);
        let len = self.glen * (lr.min(MAX_LEN) / lr);
        self.offset = -(self.ratio1 * len - len);
        self.spans[0] = r * len;
        let root = if x > 0.0 { cbrt_approx(x) } else { 0.0 };
        self.sp_root = root - self.sp_base;
        let sp = self.sp_base + self.sp_root * self.space;
        let cube = sp * (sp * sp);
        self.cube = cube as f64;
        self.spans[1] = -(r * len);
        let jscale = if ms > cube { ms as f64 } else { cube as f64 };
        self.space_common(cube, jscale);
    }

    /// `processEventInSpc` (Space).
    fn space_event(&mut self) {
        self.space = self.space_in;
        let sp = self.sp_base + self.space * self.sp_root;
        let cube = sp * (sp * sp);
        let cd = cube as f64;
        self.cube = cd;
        let jscale = if self.size_d > cd { self.size_d } else { cd };
        self.space_common(cube, jscale);
    }

    /// The part `processEventInms` and `processEventInSpc` share: feedback range, LP note,
    /// space level, jitter values and the grain period.
    fn space_common(&mut self, cube: f32, jscale: f64) {
        let c8 = cube * c(0x3c03_126f);
        let fb_lo = if c8 < 0.0 { 0.0 } else { c8 * c(0x3f19_999a) + 0.0 };
        let fb_lo = if c8 < 1.0 { fb_lo } else { c(0x3f19_999a) };
        self.lp_amt = unit(cube * c(0x3b83_126f));
        let d17 = ((cube.max(0.25) * self.sr_ms) as f64 + -1.0).max(1.0);
        self.lp.t = note_hz(self.lp_amt * self.lp_k + 96.0) as f64;
        self.fb_lo = fb_lo;
        self.fb_rng = c(0x3f73_3333) - fb_lo;
        self.fbk.t = (fb_lo + self.fb_rng * self.dens_x) as f64;
        let e = exp2_approx(((self.k154 as f64) / d17) as f32);
        self.sp_gain = e as f64;
        self.jit_scale = jscale;
        self.jit = [-(jscale * self.jit2), jscale * self.jit2];
        self.lvl_t = self.fb * e;
        let j = (self.jit_cur + self.cube).max(0.0) as f32;
        self.period = rnd(self.per_ms * ((self.size_ms + j) * 0.5));
        self.flags |= 6;
    }

    /// `GrainDelayProcessor::setDensity`.
    fn density_event(&mut self) {
        let v2 = self.dens_in + self.dens_in;
        let lo = v2.min(1.0);
        let h = (lo + -0.5) + (lo + -0.5);
        self.dens_hi.t = (v2 - lo) as f64;
        let k = h * 18.0 + 104.0 + -84.0;
        let k = if h < 0.0 { 20.0 } else { k };
        let k = if h < 1.0 { k } else { 38.0 };
        let dx = unit(lo + lo);
        self.dens_x = dx;
        self.fbk.t = (self.fb_lo + dx * self.fb_rng) as f64;
        self.lp_k = unit((dx + -0.75) * 4.0) * k + 84.0 + -96.0;
        self.lp.t = note_hz(self.lp_k * self.lp_amt + 96.0) as f64;
        self.flags |= 0xe;
    }

    /// `processFloatParameter` (parameter `idx` in processor order, stored value `v`).
    fn float_event(&mut self, idx: usize, v: f32) {
        match idx {
            0 => {
                self.pitch_in = v * 48.0;
                self.pitch.t = self.pitch_in as f64;
                self.flags |= 1;
            }
            1 => {
                self.size_in = v;
                self.size_event();
            }
            2 => {
                self.jit_in = v;
                self.jit2 = (v * v) as f64;
                self.jit = [-(self.jit_scale * self.jit2), self.jit_scale * self.jit2];
            }
            3 => {
                self.space_in = v;
                self.space_event();
            }
            4 => {
                self.dens_in = v;
                self.density_event();
            }
            5 => {
                self.mod_in = v;
                self.modu.t = (self.mod_a + v * v * self.mod_b) as f64;
                self.flags |= 0x10;
            }
            6 => {
                self.mix_in = v;
                self.mix.t = v as f64;
                self.flags |= 0x40;
            }
            _ => {
                self.stereo_in = v;
                self.stereo.t = v as f64;
                self.flags |= 0x20;
            }
        }
    }

    /// `GrainDelayProcessor::process`, per block: a smoother that has arrived (within
    /// 1e-4) stops; the control step smooths only while any still runs.
    fn block_start(&mut self) {
        let moving = [
            self.pitch.moving(),
            self.fbk.moving(),
            self.lp.moving(),
            self.dens_hi.moving(),
            self.modu.moving(),
            self.stereo.moving(),
            self.mix.moving(),
        ];
        for (bit, m) in moving.into_iter().enumerate() {
            if !m {
                self.flags &= !(1 << bit);
            }
        }
    }

    /// `processAudio` (0x101a38578): one sample.
    fn process_audio(&mut self, xl: f32, xr: f32) -> (f32, f32) {
        let w_old = self.w;
        self.w = (w_old + 1) & LINE_MASK;
        self.buf_r[w_old as usize] = xr;
        self.buf_l[w_old as usize] = xl;
        let p = self.lfo + self.lfo_inc;
        self.lfo = p - rnd(p) as f32;

        if self.ctl < self.ctl_last {
            self.ctl += 1;
            return self.tail(xl, xr, self.lvl);
        }
        // Control step.
        let g = self.grain_cnt;
        let tog = self.tog;
        self.grain_cnt = g + 1;
        let d = self.dly_s + self.dly_c * ((self.dly_t - self.dly_s) + TINY);
        self.dly = d;
        self.dly_s = d;
        let mut back_span = 0.0;
        if self.flags as i32 >= 1 {
            back_span = self.control();
        }
        // The grain clock: the other voice restarts, if its window has ended.
        let mut started = None;
        if g == 0 {
            let sel = tog ^ 1;
            self.tog = sel;
            let v = if sel >= 1 { &mut self.a } else { &mut self.b };
            if RESTART > v.win {
                v.ph = 0.0;
                v.inc = self.grain_hz * v.inv_sr;
                v.start = self.offset + w_old as i32 as f32;
                started = Some(sel);
            }
        }
        // The period alternates with the toggle: Size ± Jitter.
        let n = if tog == self.tog_seen {
            self.period
        } else {
            self.tog_seen = tog;
            let h = if tog < 0 { -0.5 } else { 0.5 };
            self.jit_cur = self.jit[((h + tog as f64) as i32 != 0) as usize];
            let j = (self.jit_cur + self.cube).max(0.0) as f32;
            self.period = rnd(self.per_ms * ((self.size_ms + j) * 0.5));
            self.period
        };
        if n <= g + 1 {
            self.grain_cnt = 0;
        }
        if self.flags as i32 >= 1 {
            self.spans[1] = back_span;
        }
        self.ctl = 0;
        let l = self.lvl_s + self.lvl_c * ((self.lvl_t - self.lvl_s) + TINY);
        self.lvl = l;
        self.lvl_s = l;
        // A voice that restarted takes the span for the Reverse setting.
        match started {
            Some(1) => self.a.rate = self.spans[self.a.rev as usize],
            Some(_) => self.b.rate = self.spans[self.b.rev as usize],
            None => {}
        }
        self.tail(xl, xr, l)
    }

    /// The smoothing part of a control step (while any flag is set). Returns the reverse
    /// span, which the kernel stores after the grain clock.
    fn control(&mut self) -> f32 {
        let m = self.mix.step();
        self.wet = (2.0 - m) * m;
        self.dry = (1.0 - m) * (2.0 - (1.0 - m));
        self.width = self.stereo.step();
        self.mod_depth = self.modu.step() + -1.0;
        let dh = self.dens_hi.step();
        let (i, f) = sat_index(dh);
        self.sat = sat_lerp(i, f);
        self.sp_wet = (2.0 - dh) * dh;
        self.up = self.up_k * dh + 1.0;
        self.wid = self.wid_k * dh + 1.0;
        let lpf = self.lp.step();
        let fb = self.fbk.step();
        self.fb = fb;
        self.lvl_t = (self.sp_gain * fb as f64) as f32;
        let pf = self.pitch.step();
        self.ratio = self.ratio_k * (exp2_approx(pf * DIV12) * NOTE0);
        let r = self.ratio;
        let lr = self.glen * r;
        self.ratio1 = r.max(1.0);
        let len = self.glen * (lr.min(MAX_LEN) / lr);
        self.offset = -(self.ratio1 * len - len);
        self.spans[0] = r * len;
        self.left.set_lp(self.band(lpf + 10.0) * self.pi_sr);
        self.right.set_lp(self.band(lpf) * self.pi_sr);
        -(r * len)
    }

    /// The per-sample part after the control step: grain reads, space lines, outputs.
    fn tail(&mut self, xl: f32, xr: f32, lvl: f32) -> (f32, f32) {
        let dly = self.dly;
        let md = self.mod_depth;
        let phb = self.b.ph;
        self.b.ph = phb + self.b.inc;
        let pha = self.a.ph;
        self.a.ph = pha + self.a.inc;
        let tri = self.lfo * (self.lfo.abs() * -8.0 + 4.0) + 0.5;
        let da = dly * (tri * md + 1.0);
        let ia = rnd(da + -0.5);
        let db = dly * ((1.0 - tri) * md + 1.0);
        let ib = rnd(db + -0.5);
        let gb = self.b.read(&self.buf_r, phb.min(1.0));
        let ga = self.a.read(&self.buf_l, pha.min(1.0));

        let ws = self.ws;
        self.sp_r[ws as usize] = self.right.fb * lvl;
        self.sp_l[ws as usize] = self.left.fb * lvl;
        let ub = ws.wrapping_sub(ib as u32);
        let r0 = self.sp_r[(ub.wrapping_sub(1) & SPACE_MASK) as usize];
        let r1 = self.sp_r[(ub & SPACE_MASK) as usize];
        let out_b = gb + (r0 + ((ib as f32 - db) + 1.0) * (r1 - r0));
        let ua = ws.wrapping_sub(ia as u32);
        let l0 = self.sp_l[(ua.wrapping_sub(1) & SPACE_MASK) as usize];
        let l1 = self.sp_l[(ua & SPACE_MASK) as usize];
        let out_a = ga + (l0 + ((ia as f32 - da) + 1.0) * (l1 - l0));

        // Each side's signal against the mid, scaled by Stereo.
        let sa = self.wid * out_a;
        let sb = out_b * self.wid;
        let mid = (sb + sa) * 0.5;
        let yr = xr * self.dry + self.wet * (mid + self.width * (sb - mid));
        let yl = xl * self.dry + self.wet * (mid + self.width * (sa - mid));

        let (up, sat, wet) = (self.up, self.sat, self.sp_wet);
        self.left.feed(out_b, up, sat, wet);
        self.right.feed(out_a, up, sat, wet);
        self.ws = (ws + 1) & SPACE_MASK;
        (yl, yr)
    }
}

/// Field names and defaults in the processor's float-parameter order (§1.5).
const FIELDS: [(&str, f64); 8] = [
    ("pitch", 0.0),
    ("size", 0.25),
    ("jitter", 0.0),
    ("space", 0.0),
    ("density", 0.25),
    ("modulation", 0.0),
    ("mix", 0.75),
    ("stereo", 0.75),
];

/// The Grain Delay effect (fields as in `crates/mxprj`; §1).
pub struct GrainDelay {
    core: Box<Core>,
    started: bool,
    /// The stored values last sent (`FIELDS` order) and Reverse.
    sent: [f32; 8],
    rev: f32,
}

impl GrainDelay {
    pub fn new(sr: f64) -> Self {
        GrainDelay { core: Box::new(Core::new(sr as f32)), started: false, sent: [0.0; 8], rev: 0.0 }
    }
}

impl Effect for GrainDelay {
    fn set(&mut self, params: &[(String, f64)]) {
        let vals = FIELDS.map(|(f, d)| param(params, f, d) as f32);
        let rev = if param(params, "reverse", 0.0) >= 0.5 { 1.0 } else { 0.0 };
        let c = &mut self.core;
        if !self.started {
            // The processor sends the stored values, then resumes (§2).
            self.started = true;
            c.pitch_in = vals[0] * 48.0;
            c.size_in = vals[1];
            c.jit_in = vals[2];
            c.space_in = vals[3];
            c.dens_in = vals[4];
            c.mod_in = vals[5];
            c.mix_in = vals[6];
            c.stereo_in = vals[7];
            c.rev_in = rev;
            c.resume();
        } else {
            for (i, v) in vals.iter().enumerate() {
                if *v != self.sent[i] {
                    c.float_event(i, *v);
                }
            }
            if rev != self.rev {
                c.rev_in = rev;
                c.rev_event();
            }
        }
        self.sent = vals;
        self.rev = rev;
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.core.block_start();
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            (*a, *b) = self.core.process_audio(*a, *b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i * 7919 + 13) % 1000) as f32 / 1000.0 - 0.5).collect()
    }

    fn run(fx: &mut GrainDelay, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut a, mut b) = (l.to_vec(), r.to_vec());
        for (x, y) in a.chunks_mut(512).zip(b.chunks_mut(512)) {
            fx.process(x, y);
        }
        (a, b)
    }

    fn p(kv: &[(&str, f64)]) -> Vec<(String, f64)> {
        kv.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn approximations_are_close() {
        assert_eq!(exp_table()[0], c(0x3f35_04f7));
        assert_eq!(log_table()[0], 0.0);
        assert_eq!(log_table()[1021], 1.0);
        for x in [0.3f32, 1.0, 5.0, 100.0] {
            assert!((log2_approx(x) - x.log2()).abs() < 1e-5, "{x}");
            assert!((cbrt_approx(x) / x.cbrt() - 1.0).abs() < 1e-3, "{x}");
        }
        assert!((note_hz(69.0) / 440.0 - 1.0).abs() < 1e-3);
        assert!((window(0.5) - 1.0).abs() < 1e-3 && window(0.0).abs() < 1e-3);
        assert!((window(0.25) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn defaults_make_grains() {
        let mut fx = GrainDelay::new(44_100.0);
        fx.set(&[]);
        let x = noise(44_100);
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        let diff: f32 = l.iter().zip(&x).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 100.0);
    }

    #[test]
    fn dry_mix_is_the_identity() {
        let mut fx = GrainDelay::new(48_000.0);
        fx.set(&p(&[("mix", 0.0), ("space", 0.7), ("pitch", 0.3)]));
        let x = noise(20_000);
        let (l, r) = run(&mut fx, &x, &x);
        assert_eq!(l, x);
        assert_eq!(r, x);
    }

    #[test]
    fn silence_stays_silent_and_loud_input_stays_bounded() {
        let mut fx = GrainDelay::new(44_100.0);
        fx.set(&p(&[("space", 1.0), ("density", 1.0), ("mix", 1.0), ("jitter", 1.0)]));
        let z = vec![0.0; 50_000];
        let (l, r) = run(&mut fx, &z, &z);
        assert!(l.iter().chain(&r).all(|v| v.abs() < 1e-12));
        let x: Vec<f32> = noise(200_000).iter().map(|v| v * 8.0).collect();
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 100.0));
    }

    #[test]
    fn set_keeps_state_and_smooths() {
        let x = noise(40_000);
        let ps = p(&[("pitch", -0.25), ("size", 0.6), ("space", 0.6), ("modulation", 0.5)]);
        let mut a = GrainDelay::new(44_100.0);
        a.set(&ps);
        let (l1, _) = run(&mut a, &x[..10_240], &x[..10_240]);
        a.set(&ps);
        let (l2, _) = run(&mut a, &x[10_240..], &x[10_240..]);
        let mut b = GrainDelay::new(44_100.0);
        b.set(&ps);
        let (lb, _) = run(&mut b, &x, &x);
        assert_eq!([l1, l2].concat(), lb);

        // A new Mix glides.
        a.set(&p(&[("pitch", -0.25), ("size", 0.6), ("space", 0.6), ("modulation", 0.5), ("mix", 0.0)]));
        run(&mut a, &x[..64], &x[..64]);
        assert!(a.core.dry > 0.0 && a.core.dry < 1.0, "{}", a.core.dry);
        run(&mut a, &x, &x);
        assert!(a.core.dry > 0.999);
    }
}
