//! Maschine's FM (docs/fx-fm.md): `RT::FMProcessor` → `FXCORE::FrequencyModulatorCore`, a
//! compiled Reaktor-Core kernel. A Q = 0.5 low-pass at Split divides each channel into a low
//! band and the rest. The low band is written at twice the sample rate into a delay line and
//! read back at a delay swept by an audio-rate oscillator (46 Hz … 8.4 kHz, from Rate) — phase
//! modulation, i.e. FM. The oscillator's depth follows Amount and, by Contour, the low band's
//! envelope. A peaking filter tuned to the oscillator then reshapes the modulated band, and it
//! is added back to the untouched rest. Left and right sweep in anti-phase. All arithmetic
//! follows the assembly operation by operation (`f32`, `f64` where the kernel uses doubles, no
//! fused multiply-adds), so the output can match bit for bit.

use super::{Effect, param};

/// Delay-line length (the kernel masks indices with 0x1fff).
const LEN: usize = 8192;
const MASK: u32 = 0x1fff;
/// The smoothers' offset that keeps them off denormals (`0x2b2bff2ee48e0530`).
const TINY_D: f64 = 1e-100;
/// The filters' offset (`0x219392ef`).
const TINY_F: f32 = f32::from_bits(0x2193_92ef);
/// Two entries of a log table the kernel interpolates at a constant position (§3.1);
/// with −61 they give the smoothers' log2 decay (as in the Chorus kernel).
const LOG_A: f32 = 0.310295;
const LOG_B: f32 = 0.311434;
const LOG_WA: f32 = f32::from_bits(0x3f75_d200); // 0.9602356
const LOG_WB: f32 = f32::from_bits(0x3d22_e000); // 0.039764404
/// The same log table at another position; with −7 they give the envelope's decay.
const ENV_A: f32 = 0.355172;
const ENV_B: f32 = 0.356276;
const ENV_WA: f32 = f32::from_bits(0x3df5_d000); // 0.120025635
const ENV_WB: f32 = f32::from_bits(0x3f61_4600); // 0.87997437
/// Coefficients of the two polynomial `2^x` approximations (§3.1).
const P3_3: f32 = f32::from_bits(0x3d65_6042); // 0.056
const P3_2: f32 = f32::from_bits(0x3e77_ced9); // 0.242
const P3_1: f32 = f32::from_bits(0x3f31_6873); // 0.693
const P2_2: f32 = f32::from_bits(0x3e7d_f3b6); // 0.248
const P2_1: f32 = f32::from_bits(0x3f35_3f7d); // 0.708
/// `log2(10)/20` (dB → log2), and 1/12 (semitones → octaves), as stored.
const DB_LOG2: f32 = f32::from_bits(0x3e2a_1512); // 0.166096
const PEAK_DB_LOG2: f32 = f32::from_bits(0x3daa_1280); // 0.0830431 (half of it: amplitude²)
const SEMI: f32 = f32::from_bits(0x3daa_a880); // 0.0833292
/// MIDI note 0 in Hz.
const NOTE0_HZ: f32 = f32::from_bits(0x4102_d6b6); // 8.17742
const TWO_PI: f32 = f32::from_bits(0x40c9_0fe4); // 6.28319
const PI: f32 = f32::from_bits(0x4049_0fd0); // 3.14159
const NINE_TENTHS: f32 = f32::from_bits(0x3f66_6666);
/// `tan` series: x + x³/3 + 2x⁵/15.
const TAN3: f32 = f32::from_bits(0x3eaa_aa9f); // 0.333333
const TAN5: f32 = f32::from_bits(0x3e08_8872); // 0.133333
/// `sin` series (x·(1 + z·(S1 + z·(S2 + …)))), z = x².
const SIN: [f32; 5] = [
    f32::from_bits(0xbe2a_aac1), // −0.166667
    f32::from_bits(0x3c08_8885), // 0.00833333
    f32::from_bits(0xb950_0c03), // −0.000198409
    f32::from_bits(0x3638_b94f), // 2.7526e-6
    f32::from_bits(0xb2cd_4ca8), // −2.39e-8
];
/// `cos` series (1 + z·(C1 + z·(C2 + …))).
const COS: [f32; 5] = [
    f32::from_bits(0xbeff_f5e8), // −0.499923
    f32::from_bits(0x3d2a_aa99), // 0.0416666
    f32::from_bits(0xbab6_09bd), // −0.00138884
    f32::from_bits(0x37cf_b5a1), // 2.47609e-5
    f32::from_bits(0xb48b_dad9), // −2.605e-7
];

const RATE_HZ: [f32; 128] = [
    46.2475, 48.1886, 50.2064, 52.3053, 54.4899, 56.7647, 59.1341, 61.6028, 64.1753, 66.8561, 69.6496, 72.5606, 75.5933, 78.7525, 82.0426,
    85.4682, 89.0337, 92.7459, 96.6381, 100.684, 104.893, 109.274, 113.835, 118.587, 123.538, 128.697, 134.073, 139.675, 145.513, 151.595,
    157.93, 164.528, 171.397, 178.547, 185.995, 193.799, 201.912, 210.352, 219.136, 228.284, 237.813, 247.742, 258.088, 268.869, 280.104,
    291.811, 304.008, 316.712, 329.943, 343.718, 358.055, 372.999, 388.647, 404.915, 421.838, 439.454, 457.799, 476.909, 496.82, 517.568,
    539.189, 561.72, 585.197, 609.656, 635.134, 661.666, 689.288, 718.038, 748.022, 779.397, 812.016, 845.952, 881.278, 918.066, 956.389,
    996.318, 1037.93, 1081.29, 1126.47, 1173.55, 1222.6, 1273.69, 1326.9, 1382.29, 1439.94, 1500.1, 1563.01, 1628.42, 1696.47, 1767.31,
    1841.08, 1917.93, 1998.01, 2081.45, 2168.41, 2259.02, 2353.44, 2451.8, 2554.26, 2660.96, 2772.03, 2887.63, 3008.34, 3134.47, 3265.63,
    3402.09, 3544.14, 3692.09, 3846.21, 4006.79, 4174.13, 4348.52, 4530.24, 4719.58, 4916.84, 5122.3, 5336.26, 5559.0, 5790.81, 6032.98,
    6285.9, 6548.88, 6822.53, 7107.4, 7404.08, 7713.15, 8035.19, 8370.79,
];

const DEPTH: [f32; 128] = [
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.970432, 0.931507, 0.894137, 0.858265, 0.823837, 0.790802, 0.759111, 0.728717, 0.699507, 0.671348,
    0.644379, 0.618529, 0.593736, 0.569944, 0.547106, 0.52518, 0.504126, 0.483911, 0.4645, 0.445865, 0.427977, 0.41081, 0.394338, 0.378536,
    0.36338, 0.348807, 0.334769, 0.321322, 0.308433, 0.29607, 0.284206, 0.272818, 0.261884, 0.251385, 0.241304, 0.231625, 0.222333,
    0.213413, 0.204852, 0.196639, 0.188759, 0.181202, 0.173932, 0.166933, 0.160228, 0.153802, 0.147637, 0.141721, 0.136042, 0.13059,
    0.125354, 0.120327, 0.115501, 0.110867, 0.106419, 0.102151, 0.0980549, 0.094126, 0.0903581, 0.086731, 0.0832413, 0.0798986, 0.0766939,
    0.07362, 0.07067, 0.0678382, 0.0651193, 0.0625086,
];

const PEAK_Q: [f32; 128] = [
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.973081, 0.934055, 0.896595, 0.860636, 0.82612,
    0.792988, 0.761185, 0.730657, 0.701354, 0.673226, 0.646226, 0.620309, 0.595431, 0.571551, 0.548629, 0.526626, 0.505505, 0.485231,
    0.465771, 0.447091, 0.42916, 0.411949, 0.395427, 0.379568, 0.364346, 0.349733, 0.335707, 0.322244, 0.30932, 0.296914, 0.285006,
    0.273576, 0.262604, 0.252072, 0.241963, 0.232259, 0.222944, 0.214003, 0.20542, 0.197182, 0.189274, 0.181683, 0.174396, 0.167402,
    0.160688, 0.154244, 0.148058, 0.14212, 0.13642, 0.130949, 0.125697, 0.120656, 0.115817, 0.111172, 0.106714, 0.102434, 0.1, 0.1, 0.1,
    0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1,
];

const PEAK_DB: [f32; 128] = [
    0.0, -0.377953, -0.755906, -1.13386, -1.51181, -1.88976, -2.26772, -2.64567, -3.02362, -3.40157, -3.77953, -4.15748, -4.53543,
    -4.91339, -5.29134, -5.66929, -6.04724, -6.4252, -6.80315, -7.1811, -7.55906, -7.93701, -8.31496, -8.69291, -9.07087, -9.44882,
    -9.82677, -10.2047, -10.5827, -10.9606, -11.3386, -11.7165, -12.0945, -12.4724, -12.8504, -13.2283, -13.6063, -13.9843, -14.3622,
    -14.7402, -15.1181, -15.4961, -15.874, -16.252, -16.6299, -17.0079, -17.3858, -17.7638, -18.1417, -18.5197, -18.8976, -19.2756,
    -19.6535, -20.0315, -20.4094, -20.7874, -21.1654, -21.5433, -21.9213, -22.2992, -22.6772, -23.0551, -23.4331, -23.811, -24.0, -24.0,
    -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0,
    -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0,
    -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0, -24.0,
    -24.0, -24.0, -24.0, -24.0, -24.0,
];

/// `round()` as the kernel does it: add ±0.5 (by the sign of the `f32`) in `f64`, truncate.
fn round_away(x: f32) -> i32 {
    let h = if x < 0.0 { -0.5 } else { 0.5 };
    (h + x as f64) as i32
}

/// `2^n` by putting `n` into the exponent bits of 1.0.
fn pow2i(n: i32) -> f32 {
    f32::from_bits(0x3f80_0000u32.wrapping_add((n as u32) << 23))
}

/// The events' `2^x`: a cubic on the fraction (§3.1).
fn exp2_p3(x: f32) -> f32 {
    let n = round_away(x);
    let f = x - n as f32;
    (f * (f * (f * P3_3 + P3_2) + P3_1) + 1.0) * pow2i(n)
}

/// The control block's `2^x`: a quadratic on the fraction (§3.1).
fn exp2_p2(x: f32) -> f32 {
    let n = round_away(x);
    let f = x - n as f32;
    (f * (f * P2_2 + P2_1) + 1.0) * pow2i(n)
}

/// `2^x` for coefficients (§3.1): a 1022-point table over one octave, linear interpolation,
/// and the integer part put into the exponent bits. The same table as the Chorus kernel's.
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
    v * pow2i(n)
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

/// Linear interpolation in a 128-entry table at `r` (0..127), with the kernel's index
/// clamps (§3.3): `(i0, i1, fraction)`.
fn lookup(r: f32) -> (usize, usize, f32) {
    let i = round_away(r + -0.5);
    let i0 = if (i as u32) < 127 { i as usize } else { 127 };
    let i1 = if (i.wrapping_add(1) as u32) >= 127 { 127 } else { (i + 1) as usize };
    (i0, i1, r - i as f32)
}

/// One linear read of a delay line `d` (half-)samples behind position `w` (§3.6).
fn tap(buf: &[f32; LEN], w: u32, d: f32) -> f32 {
    let n = round_away(d + -0.5);
    let rd = w.wrapping_sub(n as u32);
    let a = buf[(rd.wrapping_sub(1) & MASK) as usize];
    let b = buf[(rd & MASK) as usize];
    a + (b - a) * ((n as f32 - d) + 1.0)
}

/// A one-pole step of a double-precision parameter smoother.
fn smooth(s: f64, t: f64, c: f64) -> f64 {
    s + c * ((t - s) + TINY_D)
}

/// The per-channel state: split low-pass, 2× write interpolation, the modulated delay line
/// and the peaking filter (§3.4–3.7).
struct Chan {
    lp_x1: f32,
    lp_x2: f32,
    lp_y1: f32,
    lp_y2: f32,
    /// The previous low-band sample (for the half-sample write).
    prev: f32,
    /// The previous delay (for the half-sample read).
    prev_d: f32,
    pk_x1: f32,
    pk_x2: f32,
    pk_y1: f32,
    pk_y2: f32,
    buf: Box<[f32; LEN]>,
}

impl Chan {
    fn new() -> Chan {
        Chan {
            lp_x1: 0.0,
            lp_x2: 0.0,
            lp_y1: 0.0,
            lp_y2: 0.0,
            prev: 0.0,
            prev_d: 0.0,
            pk_x1: 0.0,
            pk_x2: 0.0,
            pk_y1: 0.0,
            pk_y2: 0.0,
            buf: Box::new([0.0; LEN]),
        }
    }
}

/// `FXCORE::FrequencyModulatorCore`'s state (§3).
struct Core {
    sr: f32,
    /// Event inputs: Rate, Split, Contour, Amount (normalized).
    in_rate: f32,
    in_split: f32,
    in_cont: f32,
    in_amt: f32,
    /// Control-rate update flags (+0x48). Set for good by `end_init`.
    dirty: i32,
    ctr: i32,
    period: i32,
    amt_t: f64,
    amt_s: f64,
    amt_c: f64,
    cont_t: f64,
    cont_s: f64,
    cont_c: f64,
    split_t: f64,
    split_s: f64,
    split_c: f64,
    rate_t: f64,
    rate_s: f64,
    rate_c: f64,
    amt: f32,
    cont: f32,
    /// Split clamp (Hz) and `2π/sr`.
    lo: f32,
    hi: f32,
    w_k: f32,
    /// Split low-pass: b0 (= b2), b1, a1, a2 (the a's with the sign they are added with).
    b0: f32,
    b1: f32,
    a1: f32,
    a2: f32,
    /// Oscillator: increment, phase, depth table value.
    inc: f32,
    phase: f32,
    depth_k: f32,
    /// Peaking filter: g = tan(π f/sr), its other coefficients and the output gain.
    pk_g: f32,
    pk_a1: f32,
    pk_a2: f32,
    pk_n: f32,
    pk_gain: f32,
    /// Envelope follower.
    env: f32,
    att: f32,
    rel: f32,
    /// Samples per ms at twice the rate; `π/sr`; `1/sr`.
    ms2: f32,
    pi_sr: f32,
    inv_sr: f32,
    widx: u32,
    l: Chan,
    r: Chan,
}

impl Core {
    fn new(sr: f32) -> Core {
        Core {
            sr,
            in_rate: 0.0,
            in_split: 0.0,
            in_cont: 0.0,
            in_amt: 0.0,
            dirty: 0,
            ctr: 0,
            period: 0,
            amt_t: 0.0,
            amt_s: 0.0,
            amt_c: 0.0,
            cont_t: 0.0,
            cont_s: 0.0,
            cont_c: 0.0,
            split_t: 0.0,
            split_s: 0.0,
            split_c: 0.0,
            rate_t: 0.0,
            rate_s: 0.0,
            rate_c: 0.0,
            amt: 0.0,
            cont: 0.0,
            lo: 0.0,
            hi: 0.0,
            w_k: 0.0,
            b0: 0.0,
            b1: 0.0,
            a1: 0.0,
            a2: 0.0,
            inc: 0.0,
            phase: 0.0,
            depth_k: 0.0,
            pk_g: 0.0,
            pk_a1: 0.0,
            pk_a2: 0.0,
            pk_n: 0.0,
            pk_gain: 0.0,
            env: 0.0,
            att: 0.0,
            rel: 0.0,
            ms2: 0.0,
            pi_sr: 0.0,
            inv_sr: 0.0,
            widx: 0,
            l: Chan::new(),
            r: Chan::new(),
        }
    }

    /// `processEventInAmt`: Amount → depth factor `v·10^((40v − 40)/20)`.
    fn amt_target(v: f32) -> f64 {
        let e = exp2_p3((v * 40.0 + -40.0) * DB_LOG2);
        (v * e + 0.0) as f64
    }

    /// `processEventInSpl`: Split → crossover in Hz, MIDI note `24 + 112·v`.
    fn split_target(v: f32) -> f64 {
        (exp2_p3((v * 112.0 + 24.0) * SEMI) * NOTE0_HZ) as f64
    }

    /// `processEventInCont`: Contour → how much the envelope scales the depth.
    fn cont_target(v: f32) -> f64 {
        let s = v * v;
        (NINE_TENTHS - s * NINE_TENTHS) as f64
    }

    /// `FMProcessor::resume` + `FrequencyModulatorCore::endInit` (§2, §3.2): clear
    /// everything, take the parameters as they are (no smoothing), and run one silent sample.
    fn resume(&mut self) {
        let (sr, rate, split, cont, amt) = (self.sr, self.in_rate, self.in_split, self.in_cont, self.in_amt);
        *self = Core::new(sr);
        (self.in_rate, self.in_split, self.in_cont, self.in_amt) = (rate, split, cont, amt);

        // Control rate and the smoothers' coefficients.
        let srd = sr as f64;
        let wsr = ((if sr < 0.0 { -0.5 } else { 0.5 }) + srd) as i32;
        let q = srd / 9800.0;
        let steps = ((if q < 0.0 { -0.5 } else { 0.5 }) + q) as i32;
        let steps = if (steps as u32) > 1 { steps } else { 1 };
        let per_ms = (wsr / steps) as f64 / 1000.0;
        let log = ((LOG_A * LOG_WA + LOG_B * LOG_WB) + -61.0) as f64;
        let c_amt = 1.0 - exp2_approx((log / (500.0 * per_ms + -1.0).max(1.0)) as f32) as f64;
        let c_other = 1.0 - exp2_approx((log / (100.0 * per_ms + -1.0).max(1.0)) as f32) as f64;
        self.period = steps - 1;
        self.dirty = 1;

        self.ms2 = (sr + sr) / 1000.0;
        self.lo = sr / 24576.0;
        self.hi = sr / 2.125;
        self.w_k = TWO_PI / sr;
        self.pi_sr = PI / sr;
        self.inv_sr = 1.0 / sr;

        // Envelope follower: 12 ms attack, 70 ms release (to 2^−7).
        let ms = sr / 1000.0;
        let elog = (ENV_A * ENV_WA + ENV_B * ENV_WB) + -7.0;
        self.att = 1.0 - exp2_approx(elog / (ms * 12.0 + -1.0).max(1.0));
        self.rel = 1.0 - exp2_approx(elog / (ms * 70.0 + -1.0).max(1.0));

        self.amt_t = Core::amt_target(amt);
        self.cont_t = Core::cont_target(cont);
        self.split_t = Core::split_target(split);
        self.rate_t = rate as f64;
        self.amt_c = c_amt;
        self.cont_c = c_other;
        self.split_c = c_other;
        self.rate_c = c_other;
        // The smoothers start at their targets (and step once in the forced update below).
        self.amt_s = self.amt_t;
        self.cont_s = self.cont_t;
        self.split_s = self.split_t;
        self.rate_s = self.rate_t;
        self.phase = -0.5;

        // endInit's own sample: silence in, output dropped, a forced control update.
        self.tick(0.0, 0.0, true);
        self.ctr = 0;
    }

    /// The control-rate part of `processAudio` (§3.3).
    fn control(&mut self) {
        self.amt_s = smooth(self.amt_s, self.amt_t, self.amt_c);
        self.amt = self.amt_s as f32;
        self.cont_s = smooth(self.cont_s, self.cont_t, self.cont_c);
        self.cont = self.cont_s as f32;
        self.split_s = smooth(self.split_s, self.split_t, self.split_c);
        let f = self.split_s as f32;
        self.rate_s = smooth(self.rate_s, self.rate_t, self.rate_c);
        let r = self.rate_s as f32;

        let (i0, i1, fr) = lookup(r * 127.0);
        self.depth_k = DEPTH[i0] + (DEPTH[i1] - DEPTH[i0]) * fr;
        // Peaking filter: bandwidth from PEAK_Q, gain from PEAK_DB·Amount.
        let e1 = exp2_p2((PEAK_Q[i0] + (PEAK_Q[i1] - PEAK_Q[i0]) * fr) * 0.5);
        let e2 = exp2_p2((PEAK_DB[i0] + (PEAK_DB[i1] - PEAK_DB[i0]) * fr) * self.amt * PEAK_DB_LOG2);
        let k = (e1 * e1 + -1.0) / (e1 * e2);
        self.pk_gain = k * (e2 * e2 + -1.0);
        let hz = RATE_HZ[i0] + fr * (RATE_HZ[i1] - RATE_HZ[i0]);
        let g0 = self.pi_sr * hz;
        let g2 = g0 * g0;
        let g3 = g0 * g2;
        let g5 = g2 * g3;
        let g = g0 + (g3 * TAN3 + g5 * TAN5);
        self.pk_g = g;
        let gg = g * g;
        let s = gg + 1.0;
        let gk = g * k;
        self.pk_a2 = s - gk;
        self.pk_n = 1.0 / (s + gk);
        let m = gg + -1.0;
        self.pk_a1 = m + m;
        self.inc = hz * self.inv_sr;

        // Split low-pass (Q = 0.5), from polynomial sin and cos.
        let f = if self.hi < f {
            self.hi
        } else if self.lo > f {
            self.lo
        } else {
            f
        };
        let w = f * self.w_k;
        let z = w * w;
        let mut p = z * SIN[4] + SIN[3];
        p = z * p + SIN[2];
        p = z * p + SIN[1];
        p = z * p + SIN[0];
        let sin = w * (z * p + 1.0);
        let a = 1.0 / (sin + 1.0);
        self.a2 = -((1.0 - sin) * a);
        let mut c = z * COS[4] + COS[3];
        c = z * c + COS[2];
        c = z * c + COS[1];
        c = z * c + COS[0];
        let cos = z * c + 1.0;
        self.a1 = (cos + cos) * a;
        let omc = 1.0 - cos;
        self.b1 = omc * a;
        self.b0 = (omc * 0.5) * a;
    }

    /// `FrequencyModulatorCore::processAudio` for one sample (§3.4–3.7). `init` forces the
    /// control update without touching the counter, as `endInit` does.
    fn tick(&mut self, xl: f32, xr: f32, init: bool) -> (f32, f32) {
        let w = self.widx;
        self.widx = (w + 2) & MASK;
        let (rx1, rx2, ry1, ry2) = (self.r.lp_x1, self.r.lp_x2, self.r.lp_y1, self.r.lp_y2);
        let (lx1, lx2, ly1, ly2) = (self.l.lp_x1, self.l.lp_x2, self.l.lp_y1, self.l.lp_y2);
        self.r.lp_x2 = rx1 + TINY_F;
        self.r.lp_y2 = ry1 + TINY_F;
        self.l.lp_x2 = lx1 + TINY_F;
        self.l.lp_y2 = ly1 + TINY_F;
        self.r.lp_x1 = xr + TINY_F;
        self.l.lp_x1 = xl + TINY_F;
        if init {
            self.control();
        } else if self.dirty >= 1 {
            if self.ctr < self.period {
                self.ctr += 1;
            } else {
                self.control();
                self.ctr = 0;
            }
        }

        // Split low-pass; its output is written twice per sample (§3.5).
        let wc = w.min(9999) as usize;
        let w1 = (w + 1) & MASK;
        let (b0, b1, a1, a2) = (self.b0, self.b1, self.a1, self.a2);
        let yr = (((xr * b0 + rx1 * b1) + rx2 * b0) + ry1 * a1) + ry2 * a2;
        self.r.lp_y1 = yr + TINY_F;
        self.r.buf[wc] = (self.r.prev + yr) * 0.5;
        self.r.prev = yr;
        let yl = (((xl * b0 + lx1 * b1) + lx2 * b0) + ly1 * a1) + ly2 * a2;
        self.l.lp_y1 = yl + TINY_F;
        self.l.buf[wc] = (self.l.prev + yl) * 0.5;
        self.l.prev = yl;

        // Oscillator phase in [−0.5, 0.5).
        let p = self.phase + self.inc;
        let k = round_away(p) as f32;
        let ph = p - k;
        self.phase = ph;

        let peak = if yl.abs() > yr.abs() { yl.abs() } else { yr.abs() };
        self.r.buf[w1 as usize] = yr;
        self.l.buf[w1 as usize] = yl;

        // Envelope of the low band (§3.4).
        let d = peak - self.env;
        let c = if d > 0.0 { self.att } else { self.rel };
        self.env += (d + TINY_F) * c;

        // Shape: ¾ triangle + ¼ parabolic sine, 0..1 (§3.4).
        let mut tri = (ph * 0.5).abs();
        tri = tri + tri;
        tri = tri + tri;
        let par = ph * ((p - k).abs() * -8.0 + 4.0) + 0.5;
        let shape = tri + (par - tri) * 0.25;
        let e = self.cont * (self.env * self.env + -1.0) + 1.0;
        let depth = self.depth_k * ((self.amt * e) * 30.0);
        let lfo = shape * depth;

        let ol = (xl - yl) + self.chan_out(false, w, w1, self.ms2 * lfo);
        let or = (xr - yr) + self.chan_out(true, w, w1, (depth - lfo) * self.ms2);
        (ol, or)
    }

    /// The modulated read of one channel's low band and its peaking filter (§3.6–3.7).
    fn chan_out(&mut self, right: bool, w: u32, w1: u32, d: f32) -> f32 {
        let (g, a1, a2, n, gain) = (self.pk_g, self.pk_a1, self.pk_a2, self.pk_n, self.pk_gain);
        let ch = if right { &mut self.r } else { &mut self.l };
        let avg = (ch.prev_d + d) * 0.5;
        ch.prev_d = d;
        let v = (tap(&ch.buf, w1, d) + tap(&ch.buf, w, avg)) * 0.5;
        let x2 = ch.pk_x2;
        ch.pk_x2 = ch.pk_x1;
        ch.pk_x1 = v;
        let (y1, y2) = (ch.pk_y1, ch.pk_y2);
        ch.pk_y2 = y1 + TINY_F;
        let y = n * ((g * (v - x2) - y1 * a1) - y2 * a2);
        ch.pk_y1 = y + TINY_F;
        v + gain * y
    }
}

/// The FM effect: the parameters (`rate`, `split`, `contour`, `amount`) go to the core as
/// events.
pub struct Fm {
    core: Core,
    started: bool,
}

impl Fm {
    pub fn new(sr: f64) -> Fm {
        Fm { core: Core::new(sr as f32), started: false }
    }
}

impl Effect for Fm {
    fn set(&mut self, params: &[(String, f64)]) {
        let c = &mut self.core;
        c.in_rate = param(params, "rate", 0.25) as f32;
        c.in_split = param(params, "split", 0.75) as f32;
        c.in_cont = param(params, "contour", 0.75) as f32;
        c.in_amt = param(params, "amount", 0.75) as f32;
        if !self.started {
            // The processor resumes with the stored values (§2).
            self.started = true;
            c.resume();
        } else {
            // processEventIn{Rate,Spl,Cont,Amt}: new smoother targets.
            c.rate_t = c.in_rate as f64;
            c.split_t = Core::split_target(c.in_split);
            c.cont_t = Core::cont_target(c.in_cont);
            c.amt_t = Core::amt_target(c.in_amt);
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            (*a, *b) = self.core.tick(*a, *b, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(rate: f64, split: f64, contour: f64, amount: f64) -> Vec<(String, f64)> {
        vec![("rate".into(), rate), ("split".into(), split), ("contour".into(), contour), ("amount".into(), amount)]
    }

    fn noise(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i * 7919 + 13) % 1000) as f32 / 1000.0 - 0.5).collect()
    }

    fn run(fx: &mut Fm, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut a, mut b) = (l.to_vec(), r.to_vec());
        for (x, y) in a.chunks_mut(512).zip(b.chunks_mut(512)) {
            fx.process(x, y);
        }
        (a, b)
    }

    #[test]
    fn exp2_approximations() {
        for x in [-3.0f32, -1.0, -0.25, 0.0, 0.4, 2.0] {
            assert!((exp2_approx(x) / x.exp2() - 1.0).abs() < 1e-5, "{x}");
            assert!((exp2_p3(x) / x.exp2() - 1.0).abs() < 2e-3, "{x}");
            assert!((exp2_p2(x) / x.exp2() - 1.0).abs() < 2e-2, "{x}");
        }
        // Split 0 → MIDI 24 (32.7 Hz), 1 → MIDI 136 (≈ 21 kHz).
        assert!((Core::split_target(0.0) / 32.703 - 1.0).abs() < 1e-3);
        assert!((Core::split_target(1.0) / 21096.0 - 1.0).abs() < 2e-3);
        assert!((Core::amt_target(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn defaults_modulate_the_input() {
        let mut fx = Fm::new(44_100.0);
        fx.set(&[]);
        let x = noise(44_100);
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        let diff: f32 = l.iter().zip(&x).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 100.0, "{diff}");
        // The two sides sweep in anti-phase, so they differ.
        assert!(l.iter().zip(&r).any(|(a, b)| a != b));
    }

    #[test]
    fn zero_amount_is_nearly_transparent() {
        // Amount 0: no sweep and no peak gain, so the low band comes back one sample late
        // through linear interpolation; the sum is close to the input at low frequencies.
        let mut fx = Fm::new(48_000.0);
        fx.set(&p(0.5, 0.0, 1.0, 0.0));
        let x: Vec<f32> = (0..20_000).map(|i| (i as f32 * 0.001).sin() * 0.5).collect();
        let (l, _) = run(&mut fx, &x, &x);
        let err = l.iter().zip(&x).skip(1000).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(err < 0.01, "{err}");
    }

    #[test]
    fn channels_are_independent_and_silence_stays_quiet() {
        let mut fx = Fm::new(44_100.0);
        fx.set(&p(0.9, 1.0, 0.0, 1.0));
        let x = noise(30_000);
        let z = vec![0.0; x.len()];
        let (l, r) = run(&mut fx, &x, &z);
        assert!(l.iter().any(|v| *v != 0.0));
        assert!(r.iter().all(|v| v.abs() < 1e-12));
    }

    #[test]
    fn set_keeps_state_and_smooths() {
        let x = noise(40_000);
        let mut a = Fm::new(44_100.0);
        a.set(&p(0.4, 0.6, 0.5, 0.7));
        let (l1, _) = run(&mut a, &x[..10_000], &x[..10_000]);
        a.set(&p(0.4, 0.6, 0.5, 0.7));
        let (l2, _) = run(&mut a, &x[10_000..], &x[10_000..]);
        let mut b = Fm::new(44_100.0);
        b.set(&p(0.4, 0.6, 0.5, 0.7));
        let (lb, _) = run(&mut b, &x, &x);
        assert_eq!([l1, l2].concat(), lb);

        // Amount 0 → 1 glides over several control steps.
        let mut c = Fm::new(44_100.0);
        c.set(&p(0.4, 0.6, 0.5, 0.0));
        run(&mut c, &x[..1000], &x[..1000]);
        c.set(&p(0.4, 0.6, 0.5, 1.0));
        run(&mut c, &x[..50], &x[..50]);
        assert!(c.core.amt > 0.0 && c.core.amt < 0.9, "{}", c.core.amt);
        run(&mut c, &x, &x);
        assert!((c.core.amt - 1.0).abs() < 1e-4);
    }

    #[test]
    fn loud_input_stays_bounded() {
        let mut fx = Fm::new(96_000.0);
        fx.set(&p(1.0, 0.0, 0.0, 1.0));
        let x: Vec<f32> = noise(200_000).iter().map(|v| v * 8.0).collect();
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 1000.0));
    }
}
