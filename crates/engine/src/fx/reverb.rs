//! Maschine's Reverb, shown as "Reverb (Legacy)" (docs/fx-reverb.md): `RT::ReverbProcessor`
//! → `FXCORE::FDN28Reverb`, a hand-written C++/NEON kernel. A feedback delay network of 28
//! lines in 7 groups of 4 (one NEON vector each): every line has a low shelf ("Low") and a
//! high shelf ("High") in its loop, a Householder feedback matrix (`g·(I − 2/28·11ᵀ)`) whose
//! gain `g` is set by Size, a shared quadrature LFO that modulates the delay times, and a
//! 4-point interpolated read. The left/right inputs feed alternate lanes; the lanes are
//! panned to the outputs by Stereo and Pan. Four room presets (General, Bright, Guitar,
//! Shatter) set the delay times and the filters. Everything is `f32` and follows the
//! assembly operation by operation, with `mul_add` only where the kernel fuses
//! (`fmadd`/`fmla`) and NEON's reciprocal estimate where the kernel uses it, so the output
//! can match bit for bit.

use super::{Effect, param};

/// One NEON vector: the four lines of a group.
type V = [f32; 4];
const GROUPS: usize = 7;

/// A room preset (`FDN28ReverbPreset`, 0xa8 bytes, built by `ReverbProcessor::staticInit`).
struct Preset {
    /// Low shelf: frequency and gain spread over the 28 lines (`x·(1 + spread·n/27)`).
    low_hz_spread: f32,
    low_db_spread: f32,
    /// High shelf: frequency (Hz), its spread (used ×−0.5), the gain
    /// `db + (1 − High)²·db_range` (dB), and the gain's spread (used ×2).
    high_hz: f32,
    high_hz_spread: f32,
    high_db: f32,
    high_db_range: f32,
    high_db_spread: f32,
    /// Feedback gain: `fb_min + shaped(Size)·fb_range`.
    fb_min: f32,
    fb_range: f32,
    /// Scale of the Householder term (1 in all presets).
    house: f32,
    /// Wet output gain.
    wet: f32,
    /// LFO rate (Hz) and depth (s).
    lfo_hz: f32,
    lfo_depth: f32,
    /// Index into [`SPREAD`]: the lanes' stereo positions.
    spread: usize,
    /// Delay times (s), by group and lane.
    delay: [V; GROUPS],
}

const PRESETS: [Preset; 4] = [
    // General
    Preset {
        low_hz_spread: 0.5,
        low_db_spread: 0.5,
        high_hz: 16736.0,
        high_hz_spread: 0.5,
        high_db: 0.0,
        high_db_range: -14.0,
        high_db_spread: 0.5,
        fb_min: 0.285,
        fb_range: 0.70000005,
        house: 1.0,
        wet: 4.0,
        lfo_hz: 0.36,
        lfo_depth: 0.00018000002,
        spread: 0,
        delay: [
            [0.0130842, 0.018050501, 0.0219709, 0.0263927],
            [0.0138606, 0.0185845, 0.0222154, 0.027020399],
            [0.0146055, 0.0193592, 0.0227752, 0.027446],
            [0.0155411, 0.0199122, 0.023533002, 0.0278674],
            [0.0160758, 0.020488199, 0.0240641, 0.028219901],
            [0.016754499, 0.0209642, 0.025187599, 0.0283895],
            [0.0175163, 0.021415, 0.0258498, 0.0288907],
        ],
    },
    // Bright
    Preset {
        low_hz_spread: 0.5,
        low_db_spread: 0.5,
        high_hz: 9846.205,
        high_hz_spread: 1.0,
        high_db: -0.2,
        high_db_range: -3.2,
        high_db_spread: 0.46,
        fb_min: 0.285,
        fb_range: 0.69799995,
        house: 1.0,
        wet: 4.1,
        lfo_hz: 0.36,
        lfo_depth: 0.00018000002,
        spread: 2,
        delay: [
            [0.015108599, 0.0209264, 0.0256532, 0.0288907],
            [0.016006, 0.0215632, 0.025952, 0.0288907],
            [0.0168711, 0.0224909, 0.026638001, 0.0288907],
            [0.0179636, 0.023156, 0.0275707, 0.0288907],
            [0.018590799, 0.023851201, 0.0284246, 0.0288907],
            [0.0193902, 0.0244278, 0.0288907, 0.0288907],
            [0.020291701, 0.024975501, 0.0288907, 0.0288907],
        ],
    },
    // Guitar
    Preset {
        low_hz_spread: 0.0,
        low_db_spread: 0.0,
        high_hz: 7744.043,
        high_hz_spread: 0.51,
        high_db: 0.0,
        high_db_range: -6.0,
        high_db_spread: 1.0,
        fb_min: 0.478,
        fb_range: 0.371,
        house: 1.0,
        wet: 5.15,
        lfo_hz: 0.36,
        lfo_depth: 0.00018000002,
        spread: 2,
        delay: [
            [0.016540399, 0.0203592, 0.0244803, 0.0218251],
            [0.0170978, 0.0242131, 0.023715401, 0.018311199],
            [0.0198741, 0.0286819, 0.023715401, 0.0215253],
            [0.0234682, 0.026070401, 0.026431601, 0.018103201],
            [0.0243458, 0.0239691, 0.0222285, 0.0178554],
            [0.020696301, 0.0197157, 0.0270075, 0.0176166],
            [0.0196378, 0.025163, 0.021427602, 0.017386101],
        ],
    },
    // Shatter
    Preset {
        low_hz_spread: 0.4,
        low_db_spread: 0.29,
        high_hz: 6809.627,
        high_hz_spread: 0.48,
        high_db: 0.0,
        high_db_range: -7.6,
        high_db_spread: 0.37,
        fb_min: 0.684,
        fb_range: 0.21499997,
        house: 1.0,
        wet: 5.3,
        lfo_hz: 0.36,
        lfo_depth: 0.00018000002,
        spread: 1,
        delay: [
            [0.0146418, 0.0182452, 0.0161992, 0.019123899],
            [0.015471401, 0.0178532, 0.0199652, 0.0178452],
            [0.021333, 0.0167083, 0.0171206, 0.0168976],
            [0.016894, 0.0169724, 0.0214918, 0.0182955],
            [0.0204532, 0.0187707, 0.018162, 0.017781701],
            [0.0201285, 0.0190503, 0.0159006, 0.0225949],
            [0.019123899, 0.019386299, 0.0219944, 0.023120001],
        ],
    },
];

/// The lanes' stereo positions, by `Preset::spread` (`FDN28Reverb::init`, 0x10dce6270).
const SPREAD: [V; 3] = [[1.0, -1.0, 1.0, -1.0], [1.0, 1.0, -1.0, -1.0], [1.0, 0.5, -0.5, -1.0]];

/// Line number of lane `j`, before adding the group index: `n = group + 7·j`
/// (0x104d298f0). The shelves' frequency and gain spread linearly over `n = 0..27`.
const LANE_N: V = [0.0, 7.0, 14.0, 21.0];

/// Wet-gain compensation over the feedback gain (0x104d29918), read at `19·fb`. Entry 0 is
/// the one before the table proper (index −1), reached only at `fb = 0`.
const COMP: [f32; 22] = [
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.984127, 0.968254, 0.952381, 0.888889, 0.825397, 0.730159, 0.650794, 0.507937,
    0.365079, 0.238095, 0.238095,
];

/// `1/27`: the spread step between line numbers (0x3d17b426).
const STEP27: f32 = f32::from_bits(0x3d17_b426);
/// `2/27` (0x3d97b426), part of the wet gain.
const WET_K: f32 = f32::from_bits(0x3d97_b426);
/// `log2(10)/40` (0x3daa152d): dB to the exponent of a shelf's amplitude `10^(dB/40)`.
const DB_K: f32 = f32::from_bits(0x3daa_152d);
/// The kernel's pi (0x40490fd0).
const PI: f32 = f32::from_bits(0x4049_0fd0);
/// Its tan series `x + x³/3 + 2x⁵/15`, with two spellings of 1/3.
const TAN_3: f32 = f32::from_bits(0x3eaa_aaab);
const TAN_3S: f32 = f32::from_bits(0x3eaa_aaaa);
const TAN_5: f32 = f32::from_bits(0x3e08_8886);
/// Its exp2 polynomial on the fraction: `(0.248·f + 0.708)·f + 1`.
const EXP_A: f32 = f32::from_bits(0x3e7d_f3b6);
const EXP_B: f32 = f32::from_bits(0x3f35_3f7d);
/// Low shelf frequency: `8.1758·exp(1.15525 + 1.90615·(1 − Low))` Hz.
const LOW_A: f32 = f32::from_bits(0x3f93_df14);
const LOW_B: f32 = f32::from_bits(0x3ff3_fce1);
const NOTE0_HZ: f32 = f32::from_bits(0x4102_d013);
/// The Householder term's scale, `−2/28` (0xbd924924).
const HOUSE_K: f32 = f32::from_bits(0xbd92_4924);
/// Pan's weight in the lanes' positions (0x3f34fdf4).
const PAN_K: f32 = f32::from_bits(0x3f34_fdf4);
/// Gain smoother: `ln 2 / (0.02·sr)`.
const LN2: f32 = f32::from_bits(0x3f31_7215);
/// Line lengths get `·1.001 + 1` headroom (0x3f8020c5).
const LEN_K: f32 = f32::from_bits(0x3f80_20c5);

/// `round()` of a few setup values: add ±0.5 (by the sign of the `f32`) in `f64`, truncate.
fn rnd(x: f32) -> i32 {
    let h = if x < 0.0 { -0.5 } else { 0.5 };
    (h + x as f64) as i32
}

/// The vector `2^x`: round half away from zero, `(0.248·f + 0.708)·f + 1` on the fraction
/// (no fused ops), times `2^n` built in the exponent bits.
fn exp2_v(x: f32) -> f32 {
    let n = x.round() as i32;
    let f = x - n as f32;
    let p = f * EXP_A + EXP_B;
    let p = f * p + 1.0;
    p * f32::from_bits(((n << 23) as u32).wrapping_add(0x3f80_0000))
}

/// The scalar `2^x` of the output tone filter: round half to even, fused polynomial.
fn exp2_s(x: f32) -> f32 {
    let r = x.round_ties_even();
    let n = if r < 2147483648.0 && r > -2147483648.0 { r as i32 } else { i32::MIN };
    let f = x - n as f32;
    let p = f.mul_add(EXP_A, EXP_B);
    let p = p.mul_add(f, 1.0);
    p * f32::from_bits(((n << 23) as u32).wrapping_add(0x3f80_0000))
}

/// The vector tan series (no fused ops).
fn tan_v(x: f32) -> f32 {
    let x2 = x * x;
    let x3 = x * x2;
    let x5 = x2 * x3;
    x + (x3 * TAN_3 + x5 * TAN_5)
}

/// NEON `frecpe`: an 8-bit reciprocal estimate (normal inputs; the kernel's are).
fn frecpe(x: f32) -> f32 {
    let b = x.to_bits();
    let exp = (b >> 23) & 0xff;
    if exp == 0 || exp >= 253 {
        return 1.0 / x; // outside the kernel's range
    }
    let a = 0x100 | ((b >> 15) & 0xff);
    let r = ((1u32 << 19) / (a * 2 + 1)).div_ceil(2);
    f32::from_bits((b & 0x8000_0000) | ((253 - exp) << 23) | ((r & 0xff) << 15))
}

/// `1/x` as the kernel computes it: `frecpe`, then two fused `frecps` Newton steps.
fn recip(x: f32) -> f32 {
    let e = frecpe(x);
    let e = e * (-x).mul_add(e, 2.0);
    (-x).mul_add(e, 2.0) * e
}

/// A group's two shelves (one vector per field; `FDN28Reverb` +0x1d0 + 0xa0·group).
#[derive(Clone, Copy, Default)]
struct Shelves {
    /// The group's input: last sample's delay-line reads.
    x: V,
    /// Low shelf: state, `t − 1`, `1/(t + 1)`, `t`, `A² − 1`.
    low_s: V,
    low_a: V,
    low_b: V,
    low_t: V,
    low_g: V,
    /// High shelf: state, `t − 1`, `1/(t + 1)`, `A² − 1`.
    high_s: V,
    high_a: V,
    high_b: V,
    high_g: V,
}

/// A linear ramp (start, distance to the target, progress 0..1) at the control rate.
#[derive(Clone, Copy, Default)]
struct Ramp {
    cur: V,
    start: V,
    delta: V,
    prog: V,
}

impl Ramp {
    fn tick(&mut self, step: f32) {
        for i in 0..4 {
            self.prog[i] = (step + self.prog[i]).min(1.0);
            self.cur[i] = self.start[i] + self.delta[i] * self.prog[i];
        }
    }
}

/// A group of four delay lines (`Delay4`, +0x6b0 + 0x80·group).
#[derive(Clone, Default)]
struct Lines {
    /// The preset's delay times in samples.
    base: V,
    /// The longest delay the lines hold.
    max: V,
    /// Write indices, and whether each line has wrapped yet.
    w: [i32; 4],
    wrapped: [bool; 4],
    len: [i32; 4],
    buf: [Vec<f32>; 4],
}

pub struct Reverb {
    /// The host's rate, applied by the first `set` (until then the kernel runs at 44.1 k).
    host_sr: f32,
    sr: f32,
    inv_sr: f32,
    /// The parameter values last given (`set` sends only changes to the kernel).
    last: Option<[f64; 8]>,

    // The kernel's copies of the parameters.
    room: usize,
    size: f32,
    low: f32,
    high: f32,
    pan: f32,
    stereo: f32,
    mix: f32,
    freeze: bool,
    /// The freeze state in effect (follows `freeze` after a short wait).
    frozen: bool,
    /// An input mute that nothing sets (+0xa3e).
    mute: bool,
    /// An output tone control with no parameter (+0xa70, always 0.5 = flat).
    color: f32,
    /// Wet trim (+0xa38, always 0.5).
    trim: f32,

    // Derived values.
    preset: &'static Preset,
    feedback: f32,
    /// The feedback gain in use (1 while frozen).
    fb: f32,
    house: f32,
    wet_gain: f32,
    dry: f32,
    wet_mix: f32,
    /// Samples before a freeze change takes effect: [off, on].
    freeze_wait: [i32; 2],
    freeze_count: i32,
    ctl_period: i32,
    ctl_count: i32,
    ramp_step: f32,
    /// Output matrix: each lane's weight in the left and right outputs.
    mat_l: Ramp,
    mat_r: Ramp,
    /// Input balance (lanes 0 = L, 1 = R).
    in_pan: Ramp,
    spread: V,
    lfo_depth: f32,
    lfo_inc: f32,
    phase: f32,
    shelves: [Shelves; GROUPS],
    /// Output tone filter: state and coefficients.
    tone_s: V,
    tone_c: V,
    tone_d: V,
    /// Smoothed gains [input, –, dry, wet], their targets and coefficient.
    gain: V,
    gain_t: V,
    gain_c: f32,
    lines: [Lines; GROUPS],
}

impl Reverb {
    /// `ReverbProcessor()`: the kernel's constructor defaults, then `setSampleRate(44100)`.
    /// The first [`Effect::set`] sends the parameters and then the host's sample rate.
    pub fn new(sr: f64) -> Self {
        let mut r = Reverb {
            host_sr: sr as f32,
            sr: 0.0,
            inv_sr: 0.0,
            last: None,
            room: 0,
            size: 0.5,
            low: 0.0,
            high: 0.5,
            pan: 0.0,
            stereo: 0.0,
            mix: 0.5,
            freeze: false,
            frozen: false,
            mute: false,
            color: 0.5,
            trim: 0.5,
            preset: &PRESETS[0],
            feedback: 0.0,
            fb: 0.0,
            house: 0.0,
            wet_gain: 0.0,
            dry: 0.0,
            wet_mix: 0.0,
            freeze_wait: [0; 2],
            freeze_count: 0,
            ctl_period: 1,
            ctl_count: 1,
            ramp_step: 0.0,
            mat_l: Ramp::default(),
            mat_r: Ramp::default(),
            in_pan: Ramp::default(),
            spread: [0.0; 4],
            lfo_depth: 0.0,
            lfo_inc: 0.0,
            phase: 0.0,
            shelves: [Shelves::default(); GROUPS],
            tone_s: [0.0; 4],
            tone_c: [0.0; 4],
            tone_d: [0.0; 4],
            gain: [0.0; 4],
            gain_t: [0.0; 4],
            gain_c: 0.0,
            lines: Default::default(),
        };
        r.set_sample_rate(44100.0);
        r
    }

    /// `setSampleRate`: line lengths and every coefficient from the stored parameters. It
    /// clears all state and lands the smoothers and ramps on their targets.
    fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr;
        self.inv_sr = 1.0 / sr;
        self.freeze_wait = [rnd(0.005 * sr), rnd(0.1 * sr)];

        // The lines fit the longest delay of any preset plus the LFO depth.
        let mut depth = PRESETS[0].lfo_depth;
        for p in &PRESETS[1..] {
            if depth < p.lfo_depth {
                depth = p.lfo_depth;
            }
        }
        for (g, lines) in self.lines.iter_mut().enumerate() {
            for j in 0..4 {
                let mut d = PRESETS[0].delay[g][j];
                for p in &PRESETS[1..] {
                    if d < p.delay[g][j] {
                        d = p.delay[g][j];
                    }
                }
                let m = ((depth + d) * sr).mul_add(LEN_K, 1.0) + 3.0;
                let len = (rnd(m + 1.0) + 0x13) & !0xf;
                lines.max[j] = m;
                lines.len[j] = len;
                lines.buf[j] = vec![0.0; len as usize];
            }
        }

        let t = sr * 0.02;
        self.gain_c = if t < 1.0 { LN2 } else { LN2 / t };
        let n = rnd(sr / 9800.0);
        self.ctl_period = if n > 1 { n } else { 1 };
        self.ramp_step = 1.0 / (sr / self.ctl_period as f32 * 0.05);

        self.tone();
        self.low_coefs();
        self.high_coefs();
        self.size_coefs();
        self.freeze_count = self.freeze_wait[self.freeze as usize];
        self.gain_t[0] = (!(self.frozen | self.mute)) as u8 as f32;
        self.mix_gains();
        self.gain_t[3] = self.wet_gain * self.wet_mix;
        self.pan_ramp();
        self.width(true);
        self.select_preset(self.room);

        self.freeze_count = 0;
        self.frozen = self.freeze;
        self.phase = 0.0;
        self.tone_s = [0.0; 4];
        self.gain_t[0] = (!(self.frozen | self.mute)) as u8 as f32;
        self.gain = self.gain_t;
        for s in &mut self.shelves {
            s.x = [0.0; 4];
            s.low_s = [0.0; 4];
            s.high_s = [0.0; 4];
        }
        for l in &mut self.lines {
            l.w = [0; 4];
            l.wrapped = [false; 4];
        }
        self.ctl_count = 1;
        self.mat_l.prog = [1.0; 4];
        self.mat_r.prog = [1.0; 4];
        self.in_pan.prog = [1.0; 4];
    }

    /// The output tone filter: a first-order shelf at 800 Hz / 2 kHz, flat at `color` 0.5.
    fn tone(&mut self) {
        let c = self.color.mul_add(2.0, -1.0);
        let up = c >= 0.0;
        let (hz, k): (f32, f32) = if up { (2000.0, 8.0) } else { (800.0, 12.0) };
        let g = exp2_s(c * k * DB_K);
        let mut hz = hz * g;
        if self.sr < hz {
            hz = self.sr;
        }
        let x = self.inv_sr * hz * PI;
        let x2 = x * x;
        let x3 = x * x2;
        let x5 = x2 * x3;
        let t = x + x5.mul_add(TAN_5, x3 * TAN_3S);
        self.tone_c = [t + -1.0, 1.0 / (t + 1.0), 0.0, 0.0];
        self.tone_d = [if up { -1.0 } else { 1.0 }, if up { 1.0 } else { t }, g * g + -1.0, 0.0];
    }

    /// `setBasstrap` (Low): a low shelf in every line, down to −5 dB below
    /// `8.18·exp(1.155 + 1.906·(1 − Low))` Hz at Low 0, flat at Low 1.
    fn low_coefs(&mut self) {
        let u = 1.0 - self.low;
        let hz = u.mul_add(LOW_B, LOW_A).exp() * NOTE0_HZ;
        let db = u * -5.0;
        let p = self.preset;
        let hz_step = p.low_hz_spread * hz * STEP27;
        let db_step = db * p.low_db_spread * STEP27;
        let mut hz_v = LANE_N.map(|n| hz + n * hz_step);
        let mut db_v = LANE_N.map(|n| db + n * db_step);
        for s in &mut self.shelves {
            for j in 0..4 {
                let a = exp2_v(db_v[j] * DB_K);
                let t = tan_v((hz_v[j] * recip(a)).min(self.sr) * self.inv_sr * PI);
                s.low_a[j] = t + -1.0;
                s.low_b[j] = recip(t + 1.0);
                s.low_t[j] = t;
                s.low_g[j] = a * a + -1.0;
                hz_v[j] += hz_step;
                db_v[j] += db_step;
            }
        }
    }

    /// `setDamp` (High): a high shelf in every line, `db + (1 − High)²·db_range` dB.
    fn high_coefs(&mut self) {
        let p = self.preset;
        let u = 1.0 - self.high;
        let hz_k = p.high_hz_spread * -0.5;
        let db_k = p.high_db_spread + p.high_db_spread;
        let db = (u * u).mul_add(p.high_db_range, p.high_db);
        let hz_step = p.high_hz * hz_k * STEP27;
        let db_step = db * db_k * STEP27;
        let mut hz_v = LANE_N.map(|n| p.high_hz + n * hz_step);
        let mut db_v = LANE_N.map(|n| db + n * db_step);
        for s in &mut self.shelves {
            for j in 0..4 {
                let a = exp2_v(db_v[j] * DB_K);
                let t = tan_v((hz_v[j] * a).min(self.sr) * self.inv_sr * PI);
                s.high_a[j] = t + -1.0;
                s.high_b[j] = recip(t + 1.0);
                s.high_g[j] = a * a + -1.0;
                hz_v[j] += hz_step;
                db_v[j] += db_step;
            }
        }
    }

    /// `setSize`: the feedback gain `fb_min + s'·fb_range` with `s' = u·(2 − u)`,
    /// `u = s·(1.3 − 0.3|s|)`, the Householder term, and the wet gain.
    fn size_coefs(&mut self) {
        let s = self.size;
        let u = s * s.abs().mul_add(-0.3, 1.3);
        let u = u * (2.0 - u);
        let p = self.preset;
        self.feedback = u.mul_add(p.fb_range, p.fb_min);
        self.house = self.feedback * p.house * HOUSE_K;
        self.freeze_gains();
    }

    /// The feedback in use (1 while Freeze is on) and the wet gain compensated for it.
    fn freeze_gains(&mut self) {
        let fb = if self.freeze { 1.0 } else { self.feedback };
        self.fb = fb;
        let v = if fb < 0.0 { 0.0 } else { fb };
        let v = if v > 1.0 { 19.0 } else { v * 19.0 };
        let n = rnd(v + -0.5);
        let f = v - n as f32;
        let i = (n + 1).max(0) as usize;
        let comp = COMP[i].mul_add(1.0 - f, COMP[i + 1] * f);
        self.wet_gain = self.trim * comp * self.preset.wet * WET_K;
        self.gain_t[3] = self.wet_mix * self.wet_gain;
    }

    /// `setDryWet`: dry `min(2(1 − Mix), 1)`, wet `min(2·Mix, 1)`.
    fn mix_gains(&mut self) {
        let d = 1.0 - self.mix;
        let d = d + d;
        self.dry = if d > 1.0 { 1.0 } else { d };
        let w = self.mix + self.mix;
        self.wet_mix = if w > 1.0 { 1.0 } else { w };
        self.gain_t[2] = self.dry;
    }

    /// `setSource` (Pan), first half: the input balance, ramped.
    fn pan_ramp(&mut self) {
        let r = &mut self.in_pan;
        let target = [if 0.0 > self.pan { 1.0 } else { 1.0 - self.pan }, if self.pan > 0.0 { 1.0 } else { self.pan + 1.0 }];
        for (i, t) in target.into_iter().enumerate() {
            let s = r.prog[i].mul_add(r.delta[i], r.start[i]);
            r.start[i] = s;
            r.delta[i] = t - s;
            r.prog[i] = 0.0;
            r.cur[i] = s;
        }
    }

    /// `setWidth` (Stereo), also run by Pan and the preset: each lane's position
    /// `x = clamp(spread·Stereo + 0.707·Pan, ±1)`, weights `(1−x)(3+x)/4` left and
    /// `(1+x)(3−x)/4` right, ramped. `sr_init`: the redundant `+0·Δ` of `setSampleRate`.
    fn width(&mut self, sr_init: bool) {
        let p = self.pan * PAN_K;
        for j in 0..4 {
            let x = (self.spread[j] * self.stereo + p).clamp(-1.0, 1.0);
            let l = 1.0 - x;
            let r = x + 1.0;
            let tl = l * (4.0 - l) * 0.25;
            let tr = r * (4.0 - r) * 0.25;
            for (ramp, t) in [(&mut self.mat_l, tl), (&mut self.mat_r, tr)] {
                let mut s = ramp.prog[j].mul_add(ramp.delta[j], ramp.start[j]);
                if sr_init {
                    s = 0.0f32.mul_add(t - s, s);
                }
                ramp.start[j] = s;
                ramp.delta[j] = t - s;
                ramp.prog[j] = 0.0;
                ramp.cur[j] = s;
            }
        }
    }

    /// `selectPreset` (Room): the preset's LFO, delays and lane positions, then everything
    /// that depends on it.
    fn select_preset(&mut self, room: usize) {
        self.room = room;
        let p = &PRESETS[room];
        self.preset = p;
        self.lfo_depth = p.lfo_depth * self.sr;
        self.lfo_inc = p.lfo_hz * self.inv_sr;
        for (l, d) in self.lines.iter_mut().zip(&p.delay) {
            l.base = d.map(|d| d * self.sr);
        }
        self.spread = SPREAD[p.spread];
        self.low_coefs();
        self.high_coefs();
        self.size_coefs();
        self.width(false);
    }

    /// One sample (`FDN28ReverbImpl::processAudio`).
    fn tick(&mut self, xl: f32, xr: f32) -> (f32, f32) {
        // A freeze change takes effect after its wait; then the input fades out or in.
        if self.freeze_count >= 0 {
            if self.freeze_count == 0 {
                self.frozen = self.freeze;
                self.gain_t[0] = (!(self.frozen | self.mute)) as u8 as f32;
            }
            self.freeze_count -= 1;
        }
        self.ctl_count -= 1;
        if self.ctl_count <= 0 {
            self.mat_l.tick(self.ramp_step);
            self.mat_r.tick(self.ramp_step);
            self.ctl_count = self.ctl_period;
            self.in_pan.tick(self.ramp_step);
        }

        // Quadrature parabolic-sine LFO: lanes 0/2 at phase p, 1/3 a quarter ahead.
        let p = self.phase + self.lfo_inc;
        let p = p - p.round();
        self.phase = p;
        let q = (p + 0.25) - p.round();
        let m: V = [p, q, p, q].map(|v| (v * (v.abs() * -16.0 + 8.0) + 1.0) * self.lfo_depth);

        // The shelves, on last sample's reads.
        let mut ys = [[0.0f32; 4]; GROUPS];
        let mut sum = [0.0f32; 4];
        for (g, s) in self.shelves.iter_mut().enumerate() {
            for j in 0..4 {
                let x = s.x[j];
                let a = s.low_b[j] * (x - s.low_s[j] * s.low_a[j]);
                let lo = s.low_s[j] + a;
                s.low_s[j] = a;
                let y = x + s.low_g[j] * (s.low_t[j] * lo);
                let b = s.high_b[j] * (y - s.high_s[j] * s.high_a[j]);
                let hi = s.high_g[j] * (b - s.high_s[j]);
                s.high_s[j] = b;
                let y = y + hi;
                ys[g][j] = y;
                sum[j] = if g == 0 { y + 0.0 } else { sum[j] + y };
            }
        }

        // The 28 lines to stereo, through the tone filter.
        let total = (sum[0] + sum[1]) + (sum[2] + sum[3]);
        let (ml, mr) = (&self.mat_l.cur, &self.mat_r.cur);
        let yl = (sum[0] * ml[0] + sum[1] * ml[1]) + (sum[2] * ml[2] + sum[3] * ml[3]);
        let yr = (sum[0] * mr[0] + sum[1] * mr[1]) + (sum[2] * mr[2] + sum[3] * mr[3]);
        let y4 = [yl, yr, yl, yr];
        let mut wet = [0.0f32; 2];
        for i in 0..4 {
            let s = self.tone_s[i];
            let n = (y4[i] - s * self.tone_c[0]) * self.tone_c[1];
            self.tone_s[i] = n;
            if i < 2 {
                wet[i] = y4[i] + (n + s * self.tone_d[0]) * self.tone_d[1] * self.tone_d[2];
            }
        }

        for i in 0..4 {
            self.gain[i] = self.gain[i] + self.gain_c * (self.gain_t[i] - self.gain[i]);
        }
        let inl = xl * self.gain[0];
        let inr = xr * self.gain[0];
        let bal = self.in_pan.cur;
        let out_l = wet[0] * self.gain[3] + inl * (bal[0] * self.gain[2]);
        let out_r = wet[1] * self.gain[3] + inr * (bal[1] * self.gain[2]);

        // Feed the lines: each line's output times the feedback, the Householder term, and
        // the input on alternate lanes (L,R,L,R; the next group R,L,R,L; …).
        let h = total * self.house;
        let mut feed = [bal[0] * inl + h, bal[1] * inr + h, bal[0] * inl + h, bal[1] * inr + h];
        for (g, l) in self.lines.iter_mut().enumerate() {
            for j in 0..4 {
                let v = feed[j] + ys[g][j] * self.fb;
                let d = (m[j] + l.base[j] + 3.0).max(4.0).min(l.max[j]);
                let len = l.len[j];
                let buf = &mut l.buf[j];
                buf[l.w[j] as usize] = v;
                let mut w = l.w[j] + 1;
                let wrap = w >= len;
                if wrap {
                    w -= len;
                }
                l.w[j] = w;
                l.wrapped[j] |= wrap;
                let wrapped = l.wrapped[j];
                let di = d as i32;
                let raw = w - di;
                let r = if raw < 0 { raw + len } else { raw };
                // Past the end the read continues at the start, unless nothing is there yet.
                let at = |i: i32| match r + i {
                    k if k < len => buf[k as usize],
                    k if wrapped => buf[(k - len) as usize],
                    _ => 0.0,
                };
                let (s0, s1, s2, s3) = (at(0), at(1), at(2), at(3));
                let f = d - di as f32;
                // 4-point (Catmull-Rom) interpolation, in the kernel's order.
                let c1 = (s1 - s3) * 0.5;
                let c2 = s2 - s1;
                let c0 = (s0 - s2) * 0.5;
                let a = c2 + c1;
                let c2 = c2 + a;
                let c0 = c0 + c2;
                let b = a + c0;
                let y = s2 + f * (c1 + f * (f * c0 - b));
                self.shelves[g].x[j] = if wrapped || raw >= 0 { y } else { 0.0 };
            }
            feed = [feed[1], feed[0], feed[3], feed[2]];
        }
        (out_l, out_r)
    }
}

impl Effect for Reverb {
    fn set(&mut self, params: &[(String, f64)]) {
        let v = [
            param(params, "room", 0.0),
            param(params, "size", 0.5),
            param(params, "low", 0.5),
            param(params, "high", 0.5),
            param(params, "pan", 0.0),
            param(params, "stereo", 0.0),
            param(params, "mix", 0.5),
            param(params, "freeze", 0.0),
        ];
        let first = self.last.is_none();
        let last = self.last.unwrap_or([f64::NAN; 8]);
        // Maschine's order (`connectParameters`); later calls send only what changed.
        let changed = |i: usize| first || v[i] != last[i];
        if changed(0) {
            self.select_preset((v[0].max(0.0) as usize).min(PRESETS.len() - 1));
        }
        if changed(1) {
            self.size = v[1] as f32;
            self.size_coefs();
        }
        if changed(2) {
            self.low = v[2] as f32;
            self.low_coefs();
        }
        if changed(3) {
            self.high = v[3] as f32;
            self.high_coefs();
        }
        if changed(4) {
            self.pan = v[4] as f32;
            self.pan_ramp();
            self.width(false);
        }
        if changed(5) {
            self.stereo = v[5] as f32;
            self.width(false);
        }
        if changed(6) {
            self.mix = v[6] as f32;
            self.mix_gains();
            self.gain_t[3] = self.wet_mix * self.wet_gain;
        }
        if changed(7) {
            self.freeze = v[7] != 0.0;
            self.freeze_count = self.freeze_wait[self.freeze as usize];
            self.freeze_gains();
        }
        if first {
            // The host's sample rate arrives after the parameters (`onSamplerateChanged`).
            self.set_sample_rate(self.host_sr);
        }
        self.last = Some(v);
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            (*a, *b) = self.tick(*a, *b);
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

    fn run(fx: &mut Reverb, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut a, mut b) = (l.to_vec(), r.to_vec());
        for (x, y) in a.chunks_mut(512).zip(b.chunks_mut(512)) {
            fx.process(x, y);
        }
        (a, b)
    }

    fn energy(x: &[f32]) -> f64 {
        x.iter().map(|v| (*v as f64).powi(2)).sum()
    }

    #[test]
    fn frecpe_is_neons_estimate() {
        assert_eq!(frecpe(1.0).to_bits(), 0x3f7f_8000);
        assert_eq!(frecpe(2.0).to_bits(), 0x3eff_8000);
        assert_eq!(frecpe(1.5).to_bits(), 0x3f2a_8000);
        for x in [0.3f32, 0.9, 1.0, 1.7, 3.2] {
            assert!((recip(x) * x - 1.0).abs() < 1e-6, "{x}");
        }
    }

    #[test]
    fn approximations_are_close() {
        for x in [-2.6f32, -1.0, -0.3, 0.0, 0.5, 1.4] {
            assert!((exp2_v(x) / x.exp2() - 1.0).abs() < 4e-3, "{x}");
            assert!((exp2_s(x) / x.exp2() - 1.0).abs() < 4e-3, "{x}");
        }
        for x in [0.01f32, 0.1, 0.4] {
            assert!((tan_v(x) / x.tan() - 1.0).abs() < 1e-3, "{x}");
        }
    }

    #[test]
    fn line_lengths_hold_every_preset() {
        for sr in [22_050.0, 44_100.0, 48_000.0, 96_000.0] {
            let mut fx = Reverb::new(sr);
            fx.set(&[]);
            for (g, l) in fx.lines.iter().enumerate() {
                for j in 0..4 {
                    assert_eq!(l.len[j] % 16, 0);
                    for p in &PRESETS {
                        let d = (p.delay[g][j] + p.lfo_depth) * sr as f32 + 4.0;
                        assert!(d <= l.max[j] && (l.max[j] as i32) < l.len[j] - 3);
                    }
                }
            }
        }
    }

    #[test]
    fn defaults_reverberate_and_decay() {
        let mut fx = Reverb::new(44_100.0);
        fx.set(&[]);
        let mut x = vec![0.0f32; 44_100 * 4];
        x[0] = 1.0;
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        assert_eq!(l[0], 1.0); // the dry impulse (Mix 50 %: dry and wet both at 1)
        let tail = |a: usize, b: usize| energy(&l[a..b]) + energy(&r[a..b]);
        assert!(tail(1, 4410) > 1e-6);
        assert!(tail(4410, 8820) < tail(1, 4410));
        assert!(tail(44_100 * 3, 44_100 * 4) < 1e-12);
    }

    #[test]
    fn mix_zero_is_the_identity() {
        let mut fx = Reverb::new(48_000.0);
        fx.set(&p(&[("mix", 0.0), ("size", 1.0), ("room", 2.0)]));
        let x = noise(20_000);
        let y: Vec<f32> = x.iter().map(|v| -v).collect();
        let (l, r) = run(&mut fx, &x, &y);
        assert_eq!(l, x);
        assert_eq!(r, y);
    }

    #[test]
    fn size_lengthens_the_tail() {
        let tail = |size: f64| {
            let mut fx = Reverb::new(44_100.0);
            fx.set(&p(&[("size", size), ("mix", 1.0)]));
            let mut x = vec![0.0f32; 44_100];
            x[0] = 1.0;
            let (l, r) = run(&mut fx, &x, &x);
            energy(&l[22_050..]) + energy(&r[22_050..])
        };
        assert!(tail(1.0) > 100.0 * tail(0.2));
    }

    #[test]
    fn freeze_holds_the_tail_and_mutes_the_input() {
        let mut fx = Reverb::new(44_100.0);
        fx.set(&p(&[("size", 0.5), ("mix", 1.0)]));
        let x = noise(22_050);
        run(&mut fx, &x, &x);
        fx.set(&p(&[("size", 0.5), ("mix", 1.0), ("freeze", 1.0)]));
        let (l, _) = run(&mut fx, &x, &x);
        let (l2, _) = run(&mut fx, &vec![0.0; 44_100], &vec![0.0; 44_100]);
        // Frozen: the loop gain is 1 (the shelves still shape it), so it hardly decays.
        assert!(energy(&l2[22_050..]) > 0.1 * energy(&l[11_025..]));
        assert!(l2.iter().all(|v| v.is_finite()));
        // Frozen from the start: nothing gets in, and the dry path is muted too.
        let mut fx = Reverb::new(44_100.0);
        fx.set(&p(&[("freeze", 1.0)]));
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| *v == 0.0));
    }

    #[test]
    fn one_input_reaches_both_outputs_and_silence_stays_silent() {
        let mut fx = Reverb::new(44_100.0);
        fx.set(&p(&[("mix", 1.0)]));
        let z = vec![0.0f32; 20_000];
        let (l, r) = run(&mut fx, &z, &z);
        assert!(l.iter().chain(&r).all(|v| *v == 0.0));
        let (l, r) = run(&mut fx, &noise(20_000), &z);
        assert!(energy(&l) > 1.0 && energy(&r) > 1.0);
    }

    #[test]
    fn resending_values_changes_nothing() {
        let params = p(&[("room", 1.0), ("size", 0.7), ("pan", 0.3), ("stereo", -0.5)]);
        let x = noise(30_000);
        let mut a = Reverb::new(44_100.0);
        a.set(&params);
        let (al, ar) = run(&mut a, &x, &x);
        let mut b = Reverb::new(44_100.0);
        b.set(&params);
        let (mut bl, mut br) = (x.clone(), x.clone());
        for (i, (cl, cr)) in bl.chunks_mut(1000).zip(br.chunks_mut(1000)).enumerate() {
            if i % 3 == 1 {
                b.set(&params);
            }
            b.process(cl, cr);
        }
        assert_eq!((al, ar), (bl, br));
    }

    #[test]
    fn loud_input_stays_bounded() {
        let mut fx = Reverb::new(96_000.0);
        fx.set(&p(&[("size", 1.0), ("room", 3.0), ("mix", 1.0), ("low", 1.0), ("high", 1.0)]));
        let x: Vec<f32> = noise(96_000).iter().map(|v| v * 20.0).collect();
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 1000.0));
    }
}
