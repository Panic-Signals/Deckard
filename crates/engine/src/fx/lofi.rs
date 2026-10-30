//! Maschine's Lo-Fi (docs/fx-lofi.md): `RT::LoFiProcessor` → `FXCORE::LoFiCore`, a compiled
//! Reaktor-Core kernel. A bit crusher (quantize to `2^(bits−1)` levels plus an
//! envelope-scaled noise dither, with a −102 dB gate) feeds a sample-and-hold resampler,
//! whose held value is blended by Smooth toward a linear interpolation between the last two
//! held samples. Stereo delays the right channel's hold clock by up to half a period. Levels,
//! Mix, Stereo and Smooth glide linearly over ~20 ms, updated every 128 samples. All
//! arithmetic follows the assembly operation by operation (`f32`, `mul_add` where the kernel
//! uses `fmadd`), so the output can match bit for bit.

use super::{Effect, param};

/// Control block length (`+0x178`, set by `beginInit`).
const BLOCK: i32 = 128;
/// `log2(0.01)` (`0xc0d49a78`): the envelope follower reaches 1 % after its time.
const LOG2_1PCT: f32 = f32::from_bits(0xc0d4_9a78);
/// Attack and release times of the dither envelope, in seconds (`0x104d299f0`).
const ENV_TIMES: [f32; 2] = [f32::from_bits(0x3b23_d70a), f32::from_bits(0x3cd9_e83e)];
/// The envelope's anti-denormal offset (`0x0da24260`, 1e-30).
const TINY: f32 = f32::from_bits(0x0da2_4260);
/// Input gate: where `|x| < 0.5/63095.9` (−102 dB) the crushed sample is 0 (`0x477677e6`).
const GATE: f32 = f32::from_bits(0x4776_77e6);
/// Ramp steps whose square is below this snap to the target (`1e-15`, compared in `f64`).
const SNAP: f64 = 1e-15;
/// `SR·127` with the kernel's constant (`0x42fdffff`, just below 127).
const SR_SCALE: f32 = f32::from_bits(0x42fd_ffff);
/// `2^−14`, the noise scale (`0x38800000`).
const NOISE_SCALE: f32 = f32::from_bits(0x3880_0000);

// Lanes of the ramped vector (`+0x90` current, `+0xb0` step, `+0xd0` target).
const LEVELS: usize = 0;
const CRUSH_GAIN: usize = 1;
const MIX: usize = 2;
const STEREO: usize = 3;
const SMOOTH: usize = 4;

/// The hold clock's rate in Hz by `SR·127` (`0x104d299f8`): 44100 Hz … 99.5 Hz.
const SR_HZ: [f32; 128] = [
    44100.0, 42034.7, 40066.8, 38191.3, 36403.3, 34698.0, 33070.4, 31515.6, 30040.2, 28634.7, 27293.8, 26015.0, 24795.8, 23633.7, 22526.4,
    21471.2, 20465.9, 19507.8, 18594.7, 17723.9, 16893.1, 16099.8, 15343.8, 14626.4, 13941.7, 13288.6, 12665.9, 12072.3, 11506.5, 10967.5,
    10453.9, 9964.47, 9498.04, 9053.37, 8629.22, 8224.36, 7837.57, 7470.93, 7121.37, 6787.86, 6469.81, 6166.59, 5877.59, 5602.21, 5339.81,
    5089.79, 4851.54, 4624.43, 4407.86, 4201.21, 4003.86, 3816.0, 3637.54, 3467.24, 3304.81, 3149.94, 3002.31, 2861.62, 2727.57, 2599.84,
    2478.13, 2362.13, 2251.54, 2146.04, 2045.33, 1949.11, 1858.01, 1771.06, 1688.11, 1609.01, 1533.6, 1461.73, 1393.24, 1327.99, 1265.81,
    1206.56, 1150.08, 1096.21, 1044.81, 995.721, 949.034, 904.645, 862.289, 821.892, 783.374, 746.66, 711.673, 678.335, 646.571, 616.303,
    587.454, 559.948, 533.709, 508.658, 484.742, 462.082, 440.456, 419.826, 400.153, 381.399, 363.525, 346.494, 330.266, 314.803, 300.068,
    286.021, 272.623, 259.838, 247.625, 236.023, 224.982, 214.448, 204.401, 194.821, 185.691, 176.99, 168.699, 160.8, 153.272, 146.098,
    139.257, 132.73, 126.499, 120.555, 114.919, 109.54, 104.409, 99.516,
];

/// Gain of the resampled signal by `SR·127` (`0x104d29bf8`): 1 up to index 83, then up to
/// 1.9 (it makes up for the interpolation's loss at low rates).
const SR_GAIN: [f32; 128] = [
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    1.0, 1.0, 1.0, 1.00719, 1.02859, 1.05017, 1.07183, 1.09347, 1.115, 1.13629, 1.1572, 1.1776, 1.1973, 1.21677, 1.23667, 1.257, 1.27771,
    1.29875, 1.32006, 1.34157, 1.3632, 1.38487, 1.40646, 1.42787, 1.44897, 1.46962, 1.48965, 1.50903, 1.52878, 1.54893, 1.56947, 1.59037,
    1.61157, 1.633, 1.65459, 1.67625, 1.69789, 1.71939, 1.74065, 1.76151, 1.78184, 1.80146, 1.82098, 1.84093, 1.8613, 1.88205, 1.90312,
];

/// Gain of the crushed signal by `round(Bits·127)` (`0x104d29df8`): 0.99993 … 0.2225.
const CRUSH_GAINS: [f32; 128] = [
    0.999929, 0.999887, 0.999844, 0.99983, 0.999816, 0.999804, 0.999784, 0.999769, 0.999745, 0.999724, 0.999707, 0.999684, 0.999658,
    0.999632, 0.999602, 0.999572, 0.999542, 0.999509, 0.999472, 0.999433, 0.999399, 0.999357, 0.999314, 0.999266, 0.999215, 0.999159,
    0.999107, 0.999047, 0.998986, 0.998923, 0.99885, 0.998777, 0.998701, 0.998621, 0.998522, 0.998431, 0.998307, 0.998189, 0.998082,
    0.997959, 0.997857, 0.997739, 0.997613, 0.997494, 0.997341, 0.997188, 0.996992, 0.996753, 0.996533, 0.996283, 0.996051, 0.995805,
    0.995515, 0.995212, 0.994896, 0.994549, 0.994193, 0.993822, 0.993405, 0.992949, 0.992478, 0.991962, 0.991488, 0.991032, 0.990514,
    0.989983, 0.989362, 0.988672, 0.987988, 0.987245, 0.986151, 0.983279, 0.980063, 0.976605, 0.972776, 0.968585, 0.964115, 0.959115,
    0.953746, 0.947976, 0.941635, 0.935065, 0.928043, 0.920438, 0.912232, 0.903189, 0.893304, 0.882661, 0.871559, 0.859858, 0.84797,
    0.835835, 0.823022, 0.809435, 0.794954, 0.779681, 0.763969, 0.747907, 0.731601, 0.71488, 0.697836, 0.680395, 0.662804, 0.644931,
    0.62632, 0.607443, 0.588144, 0.568694, 0.549629, 0.530731, 0.511729, 0.492688, 0.473283, 0.453761, 0.43426, 0.414631, 0.395042,
    0.375391, 0.355763, 0.33638, 0.317204, 0.298719, 0.285864, 0.273034, 0.260235, 0.247463, 0.234882, 0.222489,
];

/// `round()` as the kernel does it: add ±0.5 (by the sign of the `f32`) in `f64`, truncate.
fn round_away(x: f32) -> i32 {
    let h = if x < 0.0 { -0.5 } else { 0.5 };
    (h + x as f64) as i32
}

/// Entry `i` of a table; index −1 reads the float stored just before it, which the
/// kernel's lookup does at SR = 0 (§3.2).
fn entry(t: &[f32; 128], before: f32, i: i32) -> f32 {
    if i < 0 { before } else { t[i as usize] }
}

/// `exp2f`: Maschine calls the system libm, which `f32::exp2` also uses on macOS.
fn exp2f(x: f32) -> f32 {
    x.exp2()
}

/// `FXCORE::LoFiCore`'s state (§3).
struct Core {
    sr: f32,
    inv_sr: f32,
    /// Event inputs: SR, Bits, Smooth, Stereo, Mix (normalized).
    in_sr: f32,
    in_bits: f32,
    in_sm: f32,
    in_st: f32,
    in_mix: f32,
    /// The dither envelope's coefficients (attack, release).
    c_att: f32,
    c_rel: f32,
    /// Hold-clock rate in Hz (`+0x50`) and the resampled signal's gain (`+0x60`).
    sr_hz: f32,
    sr_gain: f32,
    /// Linear parameter ramps: current, step per sample, target (8 lanes, 5 used).
    cur: [f32; 8],
    step: [f32; 8],
    tgt: [f32; 8],
    /// An event arrived since the last control block (`+0xf0`).
    changed: bool,
    /// Control blocks left in the running ramp, and a full ramp's count.
    ramp_left: i32,
    ramp_steps: i32,
    /// `1/(ramp_steps·128)`.
    ramp_k: f32,
    /// The kernel's event state word (`+0x100`); kept for fidelity, it changes nothing.
    state: i32,
    block_ctr: i32,
    env: f32,
    phase_l: f32,
    phase_r: f32,
    /// Last sample's hold fractions + 0.5, the held samples, the ones before (L, R).
    frac: [f32; 2],
    held: [f32; 2],
    prev: [f32; 2],
    /// Noise shift register (`+0x170`).
    lfsr: u32,
}

impl Core {
    fn new(sr: f32) -> Core {
        Core {
            sr,
            inv_sr: 0.0,
            in_sr: 0.0,
            in_bits: 0.0,
            in_sm: 0.0,
            in_st: 0.0,
            in_mix: 0.0,
            c_att: 0.0,
            c_rel: 0.0,
            sr_hz: 0.0,
            sr_gain: 0.0,
            cur: [0.0; 8],
            step: [0.0; 8],
            tgt: [0.0; 8],
            changed: false,
            ramp_left: 0,
            ramp_steps: 1,
            ramp_k: 0.0,
            state: 0,
            block_ctr: 0,
            env: 0.0,
            phase_l: 0.0,
            phase_r: 0.0,
            frac: [0.0; 2],
            held: [0.0; 2],
            prev: [0.0; 2],
            // The kernel never initializes it (§2); in Maschine it starts at 0 (§5).
            lfsr: 0,
        }
    }

    /// `LoFiProcessor::resume`: `beginInit(sr)`, then `endInit()` with every event flag set
    /// (§2, §3.1). The noise register is left as it was.
    fn resume(&mut self) {
        let sr = self.sr;
        // beginInit: clear the state, envelope coefficients 1 − 2^(log2(0.01)/(t·sr − 1)).
        self.inv_sr = 1.0 / sr;
        let c = ENV_TIMES.map(|t| {
            let n = t * sr + -1.0;
            1.0 - exp2f(if n > 1.0 { LOG2_1PCT / n } else { LOG2_1PCT })
        });
        (self.c_att, self.c_rel) = (c[0], c[1]);
        self.step = [0.0; 8];
        self.tgt = [0.0; 8];
        self.changed = false;
        self.ramp_left = 0;
        self.env = 0.0;
        self.phase_l = 0.0;
        self.phase_r = 0.0;
        self.frac = [0.0; 2];
        self.held = [0.0; 2];
        self.prev = [0.0; 2];
        // The first processAudio starts a control block (counter 129 → 130 ≥ 128).
        self.block_ctr = BLOCK + 1;

        // endInit: the ramp length (~20 ms in 128-sample blocks), then every event.
        let q = (sr * 0.02) / BLOCK as f32 + 0.5;
        self.ramp_steps = (q as i32).max(1);
        self.ramp_k = 1.0 / (self.ramp_steps * BLOCK) as f32;
        self.event_sr();
        self.event_bits();
        self.tgt[STEREO] = self.in_st;
        self.tgt[SMOOTH] = self.in_sm;
        self.tgt[MIX] = self.in_mix;
        // No ramp at start: the values are taken as they are.
        self.cur = self.tgt;
        self.mark();
    }

    /// `processEventInSR`: hold-clock rate and gain, interpolated at `SR·127` (§3.2).
    fn event_sr(&mut self) {
        let x = self.in_sr * SR_SCALE;
        let i = round_away(x + -0.5);
        let f = x - i as f32;
        let (a0, a1) = (entry(&SR_HZ, ENV_TIMES[1], i), SR_HZ[(i + 1) as usize]);
        self.sr_hz = f.mul_add(a1 - a0, a0);
        let (g0, g1) = (entry(&SR_GAIN, SR_HZ[127], i), SR_GAIN[(i + 1) as usize]);
        self.sr_gain = f.mul_add(g1 - g0, g0);
    }

    /// `processEventInBits`: `levels = 2^(bits − 1)` with `bits = 12 − 11·Bits` (§3.2).
    fn event_bits(&mut self) {
        let b = self.in_bits;
        let bits = b.mul_add(-11.0, 12.0);
        self.tgt[LEVELS] = exp2f(bits + -1.0);
        self.tgt[CRUSH_GAIN] = CRUSH_GAINS[round_away(b * 127.0) as usize];
        self.mark();
    }

    /// `processEventIn{Sm,St,Mix}`: a new ramp target.
    fn event_lane(&mut self, lane: usize, v: f32) {
        self.tgt[lane] = v;
        self.mark();
    }

    fn mark(&mut self) {
        self.changed = true;
        self.state = 1;
    }

    /// The control block of `processAudio` (§3.3), every 128 samples.
    fn control(&mut self) {
        if self.changed {
            self.changed = false;
            self.state = 2;
            let mut any = false;
            for k in 0..8 {
                let d = (self.tgt[k] - self.cur[k]) * self.ramp_k;
                self.step[k] = d;
                if ((d * d) as f64) < SNAP {
                    self.step[k] = 0.0;
                    self.cur[k] = self.tgt[k];
                } else {
                    any = true;
                }
            }
            if any {
                self.ramp_left = self.ramp_steps;
            } else {
                self.state = 4;
            }
        } else {
            self.ramp_left = self.ramp_left.wrapping_sub(1);
            if self.ramp_left == 0 {
                self.cur = self.tgt;
                self.step = [0.0; 8];
                self.state = 4;
            } else if self.state == 2 {
                self.state = 3;
            } else if self.state == 4 {
                self.state = 0;
            }
        }
    }

    /// `LoFiCore::processAudio` for one sample (§3.3–3.5).
    fn tick(&mut self, xl: f32, xr: f32) -> (f32, f32) {
        for k in 0..8 {
            self.cur[k] += self.step[k];
        }
        self.block_ctr += 1;
        if self.block_ctr >= BLOCK {
            self.block_ctr = 0;
            self.control();
        }
        let lv = self.cur[LEVELS];
        let x = [xl, xr];

        // Bit crusher (§3.4); off at 12 bits (2048 levels).
        let mut y = x;
        if lv <= 2047.0 {
            let inv = 1.0 / lv;
            let (al, ar) = (xl.abs(), xr.abs());
            let pk = if al < ar { ar } else { al };
            let d = (pk - self.env) + TINY;
            let c = if pk > self.env { self.c_att } else { self.c_rel };
            self.env = c.mul_add(d, self.env);
            let s = self.lfsr;
            self.lfsr = (s << 1) | (((s >> 15) ^ (s >> 6)) & 1);
            let n = ((self.lfsr & 0x3fff) as f32).mul_add(NOISE_SCALE, -0.5);
            let n = n + n;
            let dith = (inv * 0.25) * (self.env * n);
            for (k, yk) in y.iter_mut().enumerate() {
                // ceil(x·levels) − 0.5, the ceil done as truncate-and-correct.
                let v = x[k] * lv;
                let t = if v.abs() < 16_777_216.0 { (v as i32) as f32 } else { v };
                let t = if v > t { t + 1.0 } else { t };
                // The dither enters the two sides with opposite signs.
                let dk = if k == 0 { dith } else { -dith };
                let q = ((t + -0.5) * inv + dk) * self.cur[CRUSH_GAIN];
                *yk = if 0.5 > (x[k] * GATE).abs() { 0.0 } else { q };
            }
        }

        // Sample and hold, smoothed (§3.5); off when SR is exactly 0.
        if self.in_sr != 0.0 {
            let half_st = self.cur[STEREO] * 0.5;
            let p = self.sr_hz.mul_add(self.inv_sr, self.phase_l);
            let p2 = half_st + p;
            let wrap_l = p >= 0.5;
            self.phase_l = p - if wrap_l { p.round() } else { 0.0 };
            let (wrap_r, fr_r) = if p2 >= 0.5 {
                let w = p2 - p2.round();
                let t = w <= self.phase_r;
                (t, w)
            } else {
                (false, p2)
            };
            self.phase_r = fr_r;
            let mut h = self.held;
            if wrap_l {
                self.prev[0] = self.held[0];
                h[0] = y[0];
            }
            if wrap_r {
                self.prev[1] = self.held[1];
                h[1] = y[1];
            }
            let sm = self.cur[SMOOTH];
            for k in 0..2 {
                let interp = self.prev[k] + self.frac[k] * (h[k] - self.prev[k]);
                y[k] = h[k] + (self.sr_gain * interp - h[k]) * sm;
            }
            self.frac = [self.phase_l + 0.5, fr_r + 0.5];
            self.held = h;
        }

        let mix = self.cur[MIX];
        (xl + (y[0] - xl) * mix, xr + (y[1] - xr) * mix)
    }
}

/// The Lo-Fi effect: the parameters (`sample_rate`, `bits`, `smooth`, `stereo`, `mix`) go
/// to the core as events.
pub struct LoFi {
    core: Core,
    started: bool,
}

impl LoFi {
    pub fn new(sr: f64) -> Self {
        LoFi { core: Core::new(sr as f32), started: false }
    }
}

impl Effect for LoFi {
    fn set(&mut self, params: &[(String, f64)]) {
        let c = &mut self.core;
        let v = |f: &str, d: f64| param(params, f, d) as f32;
        let sr = v("sample_rate", f32::from_bits(0x3e04_def7) as f64);
        let bits = v("bits", f32::from_bits(0x3e3d_70a4) as f64);
        let (sm, st, mix) = (v("smooth", 1.0), v("stereo", 1.0), v("mix", 1.0));
        if !self.started {
            // The processor resumes with the stored values (§2).
            self.started = true;
            (c.in_sr, c.in_bits, c.in_sm, c.in_st, c.in_mix) = (sr, bits, sm, st, mix);
            c.resume();
            return;
        }
        // processFloatParameter: a changed value goes in and sends its event.
        if sr != c.in_sr {
            c.in_sr = sr;
            c.event_sr();
        }
        if bits != c.in_bits {
            c.in_bits = bits;
            c.event_bits();
        }
        if sm != c.in_sm {
            c.in_sm = sm;
            c.event_lane(SMOOTH, sm);
        }
        if st != c.in_st {
            c.in_st = st;
            c.event_lane(STEREO, st);
        }
        if mix != c.in_mix {
            c.in_mix = mix;
            c.event_lane(MIX, mix);
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            (*a, *b) = self.core.tick(*a, *b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(sr: f64, bits: f64, smooth: f64, stereo: f64, mix: f64) -> Vec<(String, f64)> {
        [("sample_rate", sr), ("bits", bits), ("smooth", smooth), ("stereo", stereo), ("mix", mix)]
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect()
    }

    fn noise(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i * 7919 + 13) % 1000) as f32 / 1000.0 - 0.5).collect()
    }

    fn run(fx: &mut LoFi, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut a, mut b) = (l.to_vec(), r.to_vec());
        for (x, y) in a.chunks_mut(512).zip(b.chunks_mut(512)) {
            fx.process(x, y);
        }
        (a, b)
    }

    #[test]
    fn tables_span_the_documented_ranges() {
        assert_eq!(SR_HZ[0], 44100.0);
        assert!((SR_HZ[127] - 99.516).abs() < 1e-3);
        assert!(SR_HZ.windows(2).all(|w| w[1] < w[0]));
        assert!(CRUSH_GAINS.windows(2).all(|w| w[1] <= w[0]));
    }

    #[test]
    fn defaults_crush_and_resample() {
        let mut fx = LoFi::new(44_100.0);
        fx.set(&[]);
        assert_eq!(fx.core.ramp_steps, 7);
        let x = noise(44_100);
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        let diff: f32 = l.iter().zip(&x).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 100.0);
        // Stereo 100 % clocks the right side half a period later.
        assert!(l.iter().zip(&r).any(|(a, b)| a != b));
    }

    #[test]
    fn neutral_settings_and_dry_mix_are_the_identity() {
        // 12 bits (Bits 0) skips the crusher, SR exactly 0 skips the resampler.
        let x = noise(20_000);
        let mut fx = LoFi::new(48_000.0);
        fx.set(&p(0.0, 0.0, 1.0, 1.0, 1.0));
        assert_eq!(run(&mut fx, &x, &x), (x.clone(), x.clone()));
        let mut fx = LoFi::new(48_000.0);
        fx.set(&p(0.8, 0.9, 0.5, 0.5, 0.0));
        assert_eq!(run(&mut fx, &x, &x), (x.clone(), x.clone()));
    }

    #[test]
    fn crusher_quantizes_to_half_steps() {
        // Bits 1 → 1 bit, one level: the crushed value is ±0.5·gain plus the dither.
        let mut fx = LoFi::new(44_100.0);
        fx.set(&p(0.0, 1.0, 1.0, 0.0, 1.0));
        let x = noise(4096);
        let (l, _) = run(&mut fx, &x, &x);
        let g = CRUSH_GAINS[127];
        for (y, v) in l.iter().zip(&x) {
            if v.abs() > 1e-4 {
                assert!((y.abs() - 0.5 * g).abs() < 0.2 * g, "{v} → {y}");
            }
        }
    }

    #[test]
    fn channels_are_independent_and_silence_stays_silent() {
        let mut fx = LoFi::new(44_100.0);
        fx.set(&p(0.7, 0.6, 0.5, 0.0, 1.0));
        let x = noise(30_000);
        let z = vec![0.0; x.len()];
        let (l, r) = run(&mut fx, &x, &z);
        assert!(l.iter().any(|v| *v != 0.0));
        // The gate zeroes the silent side's crushed samples, dither and all.
        assert!(r.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn set_keeps_state_and_ramps() {
        let x = noise(40_000);
        let mut a = LoFi::new(44_100.0);
        a.set(&p(0.4, 0.6, 0.5, 0.3, 1.0));
        let (l1, _) = run(&mut a, &x[..10_000], &x[..10_000]);
        // Re-sending the same values changes nothing.
        a.set(&p(0.4, 0.6, 0.5, 0.3, 1.0));
        let (l2, _) = run(&mut a, &x[10_000..], &x[10_000..]);
        let mut b = LoFi::new(44_100.0);
        b.set(&p(0.4, 0.6, 0.5, 0.3, 1.0));
        let (lb, _) = run(&mut b, &x, &x);
        assert_eq!([l1, l2].concat(), lb);

        // Mix 0 → 1 glides over 7 blocks of 128 samples, then lands exactly.
        let mut c = LoFi::new(44_100.0);
        c.set(&p(0.4, 0.6, 0.5, 0.3, 0.0));
        run(&mut c, &x[..1000], &x[..1000]);
        c.set(&p(0.4, 0.6, 0.5, 0.3, 1.0));
        run(&mut c, &x[..300], &x[..300]);
        let m = c.core.cur[MIX];
        assert!(m > 0.0 && m < 1.0, "{m}");
        run(&mut c, &x[..1000], &x[..1000]);
        assert_eq!(c.core.cur[MIX], 1.0);
    }

    #[test]
    fn loud_input_stays_bounded() {
        let mut fx = LoFi::new(96_000.0);
        fx.set(&p(1.0, 1.0, 1.0, 1.0, 1.0));
        let x: Vec<f32> = noise(200_000).iter().map(|v| v * 8.0).collect();
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 40.0));
    }
}
