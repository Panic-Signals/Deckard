//! Maschine's Chorus (docs/fx-chorus.md): `RT::ChorusProcessor` → `FXCORE::ChorusCore`, a
//! compiled Reaktor-Core kernel. One parabolic-sine LFO (smoothed by a one-pole) sweeps two
//! cubic-interpolated delay lines in anti-phase: left by `depth·lfo`, right by
//! `depth·(1 − lfo)`. Rate, Amount and Mix are smoothed at a control rate of
//! `round(sr/9800)` samples. All arithmetic follows the assembly operation by operation
//! (`f32`, `f64` where the kernel uses doubles, no fused multiply-adds), so the output can
//! match bit for bit.

use super::{Effect, param};

/// Delay-line length (the kernel masks indices with 0x1fff).
const LEN: usize = 8192;
const MASK: u32 = 0x1fff;
/// The smoothers' offset that keeps them off denormals (`0x2b2bff2ee48e0530`).
const TINY_D: f64 = 1e-100;
/// The LFO smoother's offset (`0x219392ef`).
const TINY_F: f32 = f32::from_bits(0x2193_92ef);
/// LFO smoother scale (`0x38daf153`), applied with `44100/sr`.
const SMOOTH_K: f32 = f32::from_bits(0x38da_f153);
/// Two entries of a log table the kernel interpolates at a constant position (§3.1);
/// with −61 they give the smoothers' log2 decay.
const LOG_A: f32 = 0.310295;
const LOG_B: f32 = 0.311434;
const LOG_WA: f32 = f32::from_bits(0x3f75_d200); // 0.9602356
const LOG_WB: f32 = f32::from_bits(0x3d22_e000); // 0.039764404

const RATE_HZ: [f32; 128] = [
    0.0639015, 0.0667532, 0.0697329, 0.0728461, 0.076098, 0.0794942, 0.0830398, 0.0867404, 0.0906037, 0.0946646, 0.0988971, 0.103312,
    0.10792, 0.112732, 0.117758, 0.12301, 0.128498, 0.134232, 0.140224, 0.146485, 0.153024, 0.159853, 0.166982, 0.174422, 0.182198,
    0.190362, 0.198871, 0.207747, 0.217013, 0.226689, 0.236796, 0.247357, 0.258393, 0.269925, 0.281974, 0.294563, 0.307712, 0.321443,
    0.335778, 0.350737, 0.366388, 0.382798, 0.399906, 0.417753, 0.436384, 0.455841, 0.476166, 0.497404, 0.519596, 0.542786, 0.567017,
    0.592331, 0.618771, 0.646381, 0.675203, 0.70528, 0.73678, 0.769769, 0.804163, 0.840048, 0.87751, 0.916634, 0.957507, 1.00021, 1.04484,
    1.09148, 1.1402, 1.19111, 1.24427, 1.29979, 1.35774, 1.41821, 1.48161, 1.54793, 1.61707, 1.68923, 1.76455, 1.84323, 1.92542, 2.0113,
    2.10105, 2.19483, 2.29281, 2.39517, 2.50208, 2.6137, 2.73022, 2.85181, 2.97939, 3.11271, 3.25174, 3.39681, 3.54828, 3.70649, 3.87177,
    4.04447, 4.22495, 4.41353, 4.61056, 4.81639, 5.03136, 5.25581, 5.49009, 5.73454, 5.99131, 6.25933, 6.53885, 6.83055, 7.13512, 7.45325,
    7.78562, 8.13292, 8.49584, 8.87507, 9.27129, 9.68518, 10.1174, 10.5687, 11.0398, 11.5313, 12.048, 12.5868, 13.1488, 13.7353, 14.3478,
    14.9875, 15.6559, 16.3543,
];

const SMOOTH: [f32; 128] = [
    1.47501, 1.54084, 1.6096, 1.68144, 1.75647, 1.83486, 1.91675, 2.00229, 2.09165, 2.18499, 2.2825, 2.38437, 2.49077, 2.60193, 2.71805,
    2.83935, 2.96606, 3.09843, 3.23671, 3.38116, 3.53205, 3.68968, 3.85434, 4.02635, 4.20604, 4.39374, 4.58983, 4.79466, 5.00864, 5.23216,
    5.46566, 5.70958, 5.96439, 6.23056, 6.50862, 6.79909, 7.10251, 7.41948, 7.7506, 8.09649, 8.45782, 8.83527, 9.22957, 9.64147, 10.0717,
    10.5212, 10.9908, 11.4813, 11.9936, 12.5289, 13.088, 13.6721, 14.2823, 14.9197, 15.5855, 16.281, 17.0076, 17.7666, 18.5595, 19.3878,
    20.253, 21.1569, 22.1011, 23.0874, 24.1177, 25.194, 26.3184, 27.4929, 28.7199, 30.0016, 31.3405, 32.7391, 34.2002, 35.7265, 37.3209,
    38.9864, 40.7263, 42.5438, 44.4425, 46.4258, 48.4977, 50.6621, 52.923, 55.2848, 57.7521, 60.3294, 63.0218, 65.8343, 68.7724, 71.8415,
    75.0477, 78.3969, 81.8955, 85.5504, 89.3683, 93.3566, 97.5229, 101.875, 106.422, 111.171, 116.132, 121.315, 126.729, 132.385, 138.293,
    144.464, 150.911, 157.646, 164.682, 172.031, 179.708, 187.728, 196.106, 204.858, 214.001, 223.551, 233.527, 243.949, 254.836, 266.209,
    278.089, 290.5, 303.464, 317.007, 331.154, 345.933, 361.371, 377.499,
];

const DEPTH_MS: [f32; 128] = [
    2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0,
    2.0, 2.0, 2.0, 2.0, 2.0, 1.97822, 1.89371, 1.81281, 1.73536, 1.66122, 1.59025, 1.52232, 1.45728, 1.39503, 1.33543, 1.27838, 1.22376,
    1.17148, 1.12144, 1.07353, 1.02766, 0.983761, 0.941733, 0.901501, 0.862988, 0.82612, 0.790827, 0.757042, 0.7247, 0.69374, 0.664103,
    0.635731, 0.608572, 0.582573, 0.557685, 0.53386, 0.511053, 0.48922, 0.46832, 0.448313, 0.42916, 0.410826, 0.393275, 0.376474, 0.36039,
    0.344994, 0.330256, 0.316147, 0.30264, 0.289711, 0.277334, 0.265486, 0.254144, 0.243287, 0.232894, 0.222944, 0.21342, 0.204302,
    0.195574, 0.187219, 0.179221, 0.171564, 0.164235, 0.157218, 0.150502, 0.144072, 0.137917, 0.132025, 0.126385, 0.120986, 0.115817,
    0.110869, 0.106133, 0.101599, 0.0972581, 0.0931032, 0.0891257, 0.0853181, 0.0816732, 0.078184, 0.0748439, 0.0716465, 0.0685857,
    0.0656556, 0.0628507, 0.0625, 0.0625, 0.0625, 0.0625, 0.0625, 0.0625, 0.0625, 0.0625, 0.0625, 0.0625, 0.0625, 0.0625, 0.0625, 0.0625,
    0.0625, 0.0625,
];

/// `round()` as the kernel does it: add ±0.5 (by the sign of the `f32`) in `f64`, truncate.
fn round_away(x: f32) -> i32 {
    let h = if x < 0.0 { -0.5 } else { 0.5 };
    (h + x as f64) as i32
}

/// `2^x` (§3.1): a 1022-point table over one octave, linear interpolation, and the
/// integer part put into the exponent bits.
fn exp2_approx(x: f32) -> f32 {
    let n = {
        // fcmge/mvn/bsl: −0.5 unless x ≥ 0.
        let h = if x >= 0.0 { 0.5 } else { -0.5 };
        (h + x as f64) as i32
    };
    let y = ((x - n as f32) + 0.5) * 1021.0;
    let z = y + -0.5;
    let k = {
        let h = if z >= 0.0 { 0.5 } else { -0.5 };
        (h + z as f64) as i32
    };
    let fr = y - k as f32;
    let i = (k as u32 & 0x3ff) as usize;
    let v = (1.0 - fr) * exp2_entry(i) + fr * exp2_entry(i + 1);
    v * f32::from_bits(0x3f80_0000u32.wrapping_add((n as u32) << 23))
}

/// Entry `i` of the kernel's octave table: `2^(i/1021 − 0.5)` to 6 significant digits,
/// with the last entry stored as √2 and seven near-ties rounded the other way (§3.1).
fn exp2_entry(i: usize) -> f32 {
    match i {
        106 => return 0.759867,
        213 => return 0.817119,
        225 => return 0.823803,
        287 => return 0.859218,
        324 => return 0.881074,
        382 => return 0.91646,
        673 => return 1.11663,
        1022.. => return f32::from_bits(0x3fb5_04d5), // 1.41421
        _ => {}
    }
    let v = (i as f64 / 1021.0 - 0.5).exp2();
    let p = 10f64.powi(5 - v.log10().floor() as i32);
    ((v * p).round() / p) as f32
}

/// Linear interpolation in one of the 128-entry tables at `r` (§3.3): returns
/// `(t[i0], t[i1] − t[i0])` with the kernel's index clamps, and the fraction.
fn lookup(r: f32) -> (usize, usize, f32) {
    let i = round_away(r + -0.5);
    let i0 = if (i as u32) < 127 { i as usize } else { 127 };
    let i1 = if (i.wrapping_add(1) as u32) >= 127 { 127 } else { (i + 1) as usize };
    (i0, i1, r - i as f32)
}

/// One cubic read of a delay line (§3.5) at `d` samples behind write position `w`.
fn tap(buf: &[f32; LEN], w: u32, d: f32) -> f32 {
    let n = round_away(d + -0.5);
    let rd = w.wrapping_sub(n as u32);
    let a = buf[(rd.wrapping_add(8190) & MASK) as usize];
    let b = buf[(rd.wrapping_sub(1) & MASK) as usize];
    let c = buf[(rd.wrapping_add(8189) & MASK) as usize];
    let e = buf[(rd & MASK) as usize];
    let t = (n as f32 - d) + 1.0;
    let c1 = (b - c) * 0.5;
    let c2 = (e - a) * 0.5;
    let ab = a - b;
    let s = ab + c1;
    let q = ab + (c2 + s);
    let u = t * ((q * t) - (s + q));
    a + t * (c1 + u)
}

/// A one-pole step of a double-precision parameter smoother.
fn smooth(s: f64, t: f64, c: f64) -> f64 {
    s + c * ((t - s) + TINY_D)
}

/// `FXCORE::ChorusCore`'s state (§3).
struct Core {
    sr: f32,
    /// Event inputs: Rate, Amount, Mix (normalized).
    in_rate: f32,
    in_amt: f32,
    in_mix: f32,
    /// Control-rate update flags (+0x3c). Set for good by `end_init`.
    dirty: i32,
    ctr: i32,
    period: i32,
    rate_t: f64,
    rate_s: f64,
    rate_c: f64,
    amt_t: f64,
    amt_s: f64,
    amt_c: f64,
    mix_t: f64,
    mix_s: f64,
    mix_c: f64,
    dry: f32,
    wet: f32,
    inv_sr: f32,
    inc: f32,
    phase: f32,
    lfo: f32,
    k44: f32,
    lfo_c: f32,
    depth: f32,
    ms: f32,
    widx: u32,
    buf_l: Box<[f32; LEN]>,
    buf_r: Box<[f32; LEN]>,
}

impl Core {
    /// `ChorusProcessor()`: a zeroed core with the processor's own input defaults.
    fn new(sr: f32) -> Core {
        Core { in_rate: 0.45, in_amt: 0.55, in_mix: 0.5, ..Core::zeroed(sr) }
    }

    fn zeroed(sr: f32) -> Core {
        Core {
            sr,
            in_rate: 0.0,
            in_amt: 0.0,
            in_mix: 0.0,
            dirty: 0,
            ctr: 0,
            period: 0,
            rate_t: 0.0,
            rate_s: 0.0,
            rate_c: 0.0,
            amt_t: 0.0,
            amt_s: 0.0,
            amt_c: 0.0,
            mix_t: 0.0,
            mix_s: 0.0,
            mix_c: 0.0,
            dry: 0.0,
            wet: 0.0,
            inv_sr: 0.0,
            inc: 0.0,
            phase: 0.0,
            lfo: 0.0,
            k44: 0.0,
            lfo_c: 0.0,
            depth: 0.0,
            ms: 0.0,
            widx: 0,
            buf_l: Box::new([0.0; LEN]),
            buf_r: Box::new([0.0; LEN]),
        }
    }

    /// `ChorusProcessor::resume` + `ChorusCore::endInit` (§2, §3.2): clear everything, take
    /// the parameters as they are (no smoothing), and run one silent sample.
    fn resume(&mut self) {
        // The processor zeroes the whole core and restores the inputs and sample rate.
        let (sr, rate, amt, mix) = (self.sr, self.in_rate, self.in_amt, self.in_mix);
        *self = Core { in_rate: rate, in_amt: amt, in_mix: mix, ..Core::zeroed(sr) };

        // Control rate and the smoothers' coefficients.
        let log = (LOG_A * LOG_WA + LOG_B * LOG_WB) + -61.0;
        let srd = sr as f64;
        let wsr = ((if sr < 0.0 { -0.5 } else { 0.5 }) + srd) as i32;
        let q = srd / 9800.0;
        let steps = ((if q < 0.0 { -0.5 } else { 0.5 }) + q) as i32;
        let ctl = if steps == 0 { 0 } else { wsr / steps };
        let per_ms = ctl as f64 / 1000.0;
        let slow = (500.0 * per_ms + -1.0).max(1.0);
        let fast = (50.0 * per_ms + -1.0).max(1.0);
        self.dirty = 1;
        self.period = steps - 1;
        self.ms = sr / 1000.0;
        self.k44 = 44100.0 / sr;
        self.inv_sr = 1.0 / sr;
        self.mix_t = mix as f64;
        let logd = log as f64;
        self.mix_c = 1.0 - exp2_approx((logd / fast) as f32) as f64;
        let c_slow = 1.0 - exp2_approx((logd / slow) as f32) as f64;
        self.phase = -0.5;
        self.rate_c = c_slow;
        self.amt_c = c_slow;
        self.rate_t = (rate * 127.0) as f64;
        self.amt_t = amt as f64;
        self.rate_s = smooth(self.rate_t, self.rate_t, self.rate_c);
        self.mix_s = smooth(self.mix_t, self.mix_t, self.mix_c);
        self.amt_s = smooth(self.amt_t, self.amt_t, self.amt_c);
        self.control();
        // endInit's own sample: silence in, output dropped, no control step.
        self.dirty = 0;
        self.tick(0.0, 0.0);
        // The flags of all three parameters end up set (§3.2), so the control-rate update
        // runs from now on.
        self.dirty = 7;
        self.ctr = 0;
    }

    /// The control-rate part of `processAudio` (§3.3), after the smoothers have stepped.
    fn control(&mut self) {
        let m = self.mix_s as f32;
        self.wet = (2.0 - m) * m;
        let dm = 1.0 - m;
        self.dry = dm * (2.0 - dm);
        let a = self.amt_s as f32;
        let r = self.rate_s as f32;
        let scale = (a * a) * 25.0;
        let (i0, i1, fr) = lookup(r);
        self.depth = scale * (DEPTH_MS[i0] + (DEPTH_MS[i1] - DEPTH_MS[i0]) * fr);
        self.lfo_c = self.k44 * ((SMOOTH[i0] + (SMOOTH[i1] - SMOOTH[i0]) * fr) * SMOOTH_K);
        self.inc = self.inv_sr * (RATE_HZ[i0] + fr * (RATE_HZ[i1] - RATE_HZ[i0]));
    }

    /// `ChorusCore::processAudio` for one sample (§3.4–3.5).
    fn tick(&mut self, xl: f32, xr: f32) -> (f32, f32) {
        let w = self.widx;
        self.widx = (w + 1) & MASK;
        self.buf_r[w as usize] = xr;
        self.buf_l[w as usize] = xl;
        if self.dirty >= 1 {
            if self.ctr < self.period {
                self.ctr += 1;
            } else {
                self.ctr = 0;
                self.mix_s = smooth(self.mix_s, self.mix_t, self.mix_c);
                self.amt_s = smooth(self.amt_s, self.amt_t, self.amt_c);
                self.rate_s = smooth(self.rate_s, self.rate_t, self.rate_c);
                self.control();
            }
        }
        // LFO: a phase in [−0.5, 0.5), shaped to x·(4 − 8|x|) + 0.5 and smoothed.
        let p = self.phase + self.inc;
        let k = round_away(p) as f32;
        let ph = p - k;
        let shape = ph * ((p - k).abs() * -8.0 + 4.0) + 0.5;
        self.lfo = self.lfo + self.lfo_c * ((shape - self.lfo) + TINY_F);
        self.phase = ph;

        let sweep = self.depth * self.lfo;
        let yl = tap(&self.buf_l, w, self.ms * sweep);
        let yr = tap(&self.buf_r, w, self.ms * (self.depth - sweep));
        (xl * self.dry + self.wet * yl, xr * self.dry + self.wet * yr)
    }
}

/// The Chorus effect: the parameters (`rate`, `amount`, `mix`) go to the core as events.
pub struct Chorus {
    core: Core,
    started: bool,
}

impl Chorus {
    pub fn new(sr: f64) -> Chorus {
        Chorus { core: Core::new(sr as f32), started: false }
    }
}

impl Effect for Chorus {
    fn set(&mut self, params: &[(String, f64)]) {
        let c = &mut self.core;
        c.in_rate = param(params, "rate", 0.5) as f32;
        c.in_amt = param(params, "amount", 0.5) as f32;
        c.in_mix = param(params, "mix", 0.5) as f32;
        if !self.started {
            // The processor resumes with the stored values (§2).
            self.started = true;
            c.resume();
        } else {
            // processEventIn{Rate,Amt,Mix}: new smoother targets.
            c.rate_t = (c.in_rate * 127.0) as f64;
            c.amt_t = c.in_amt as f64;
            c.mix_t = c.in_mix as f64;
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

    fn p(rate: f64, amount: f64, mix: f64) -> Vec<(String, f64)> {
        vec![("rate".into(), rate), ("amount".into(), amount), ("mix".into(), mix)]
    }

    fn noise(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i * 7919 + 13) % 1000) as f32 / 1000.0 - 0.5).collect()
    }

    fn run(fx: &mut Chorus, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
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
    fn rate_table_runs_over_eight_octaves() {
        assert_eq!(lookup(0.0), (127, 0, 1.0)); // the kernel's wrap at index −1
        assert_eq!(lookup(63.5), (63, 64, 0.5));
        assert!((RATE_HZ[127] / RATE_HZ[0] - 256.0).abs() < 0.2);
    }

    #[test]
    fn defaults_chorus_the_input() {
        let mut fx = Chorus::new(44_100.0);
        fx.set(&[]);
        let x = noise(44_100);
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        let diff: f32 = l.iter().zip(&x).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 100.0);
        // The two sides sweep in anti-phase, so they differ.
        assert!(l.iter().zip(&r).any(|(a, b)| a != b));
    }

    #[test]
    fn dry_mix_is_the_identity() {
        let mut fx = Chorus::new(48_000.0);
        fx.set(&p(0.7, 1.0, 0.0));
        let x = noise(20_000);
        let (l, r) = run(&mut fx, &x, &x);
        assert_eq!(l, x);
        assert_eq!(r, x);
    }

    #[test]
    fn channels_are_independent_and_silence_stays_silent() {
        let mut fx = Chorus::new(44_100.0);
        fx.set(&p(0.9, 1.0, 1.0));
        let x = noise(30_000);
        let z = vec![0.0; x.len()];
        let (l, r) = run(&mut fx, &x, &z);
        assert!(l.iter().any(|v| *v != 0.0));
        assert!(r.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn set_keeps_state_and_smooths() {
        let x = noise(40_000);
        let mut a = Chorus::new(44_100.0);
        a.set(&p(0.4, 0.6, 0.5));
        let (l1, _) = run(&mut a, &x[..10_000], &x[..10_000]);
        // Re-sending the same values changes nothing: the LFO and delay lines go on.
        a.set(&p(0.4, 0.6, 0.5));
        let (l2, _) = run(&mut a, &x[10_000..], &x[10_000..]);
        let mut b = Chorus::new(44_100.0);
        b.set(&p(0.4, 0.6, 0.5));
        let (lb, _) = run(&mut b, &x, &x);
        assert_eq!([l1, l2].concat(), lb);

        // Mix 0 → 1 glides over several control steps instead of jumping.
        let mut c = Chorus::new(44_100.0);
        c.set(&p(0.4, 0.6, 0.0));
        run(&mut c, &x[..1000], &x[..1000]);
        c.set(&p(0.4, 0.6, 1.0));
        run(&mut c, &x[..10], &x[..10]);
        assert!(c.core.dry > 0.5 && c.core.dry < 1.0, "{}", c.core.dry);
        run(&mut c, &x, &x);
        assert!(c.core.dry < 1e-6);
    }

    #[test]
    fn loud_input_stays_bounded() {
        let mut fx = Chorus::new(96_000.0);
        fx.set(&p(1.0, 1.0, 1.0));
        let x: Vec<f32> = noise(200_000).iter().map(|v| v * 8.0).collect();
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 20.0));
    }
}
