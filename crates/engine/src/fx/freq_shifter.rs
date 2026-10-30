//! Maschine's Freq Shifter (docs/fx-freq-shifter.md): `RT::FreqShifterProcessor` →
//! `FXCORE::FreqShifterCore`, a compiled Reaktor-Core kernel. Per channel, two cascades of six
//! first-order all-passes (a Hilbert pair, 90° apart) are multiplied by a parabolic
//! quadrature oscillator and summed (shift up) or differenced (shift down, "Invert"). The shifted
//! signal feeds back into the all-pass input. The right channel's oscillator runs `Stereo³/2`
//! cycles ahead of the left. Mix, Invert, Stereo, Feedback, gain and frequency are smoothed at a
//! control rate of `round(sr/9800)` samples. All arithmetic follows the assembly operation by
//! operation (`f32`, `f64` where the kernel uses doubles, no fused multiply-adds), so the output
//! can match bit for bit.

use super::{Effect, param};

/// The all-pass and feedback states' offset that keeps them off denormals (`0x219392ef`).
const TINY: f32 = f32::from_bits(0x2193_92ef);
/// The smoothers' offset (`0x2b2bff2ee48e0530`).
const TINY_D: f64 = f64::from_bits(0x2b2b_ff2e_e48e_0530);
/// `processEventInBfrE`'s convergence threshold (`0x3f1a36e2eb1c432d`, 1e-4).
const EPS: f64 = f64::from_bits(0x3f1a_36e2_eb1c_432d);

/// All-pass corner scale `15π` (`0x423c7ee0`), divided by `sr`.
const W_SCALE: f32 = f32::from_bits(0x423c_7ee0);
/// Corner factors of the two all-pass cascades (§3.1): "A" (the cosine path) and "B" (sine).
const K_A: [u32; 6] = [0x3fa0_4ea5, 0x40b2_25af, 0x41b2_bd08, 0x42b3_4113, 0x43b6_653f, 0x452d_21c3];
const K_B: [u32; 6] = [0x3eb8_c7e3, 0x402f_6fd2, 0x4132_844d, 0x4233_084b, 0x4333_9fbe, 0x4447_9d50];

/// Two log-table entries per end of the Coarse range, read at constant positions (§3.2).
const HI_A: f32 = f32::from_bits(0x3e3c_8fb8); // 0.184142
const HI_B: f32 = f32::from_bits(0x3e3d_d591); // 0.185385
const HI_WA: f32 = f32::from_bits(0x3f45_d200); // 0.7727356
const HI_WB: f32 = f32::from_bits(0x3e68_b800); // 0.2272644
const LO_A: f32 = f32::from_bits(0x3f4d_b318); // 0.803514
const LO_B: f32 = f32::from_bits(0x3f4d_e82d); // 0.804324
const LO_WA: f32 = f32::from_bits(0x3f64_1400); // 0.8909302
const LO_WB: f32 = f32::from_bits(0x3ddf_6000); // 0.109069824
/// The smoothers' log2 decay: two entries of a log table, as in the Chorus kernel.
const LOG_A: f32 = f32::from_bits(0x3e9e_defc); // 0.310295
const LOG_B: f32 = f32::from_bits(0x3e9f_7447); // 0.311434
const LOG_WA: f32 = f32::from_bits(0x3f75_d200); // 0.9602356
const LOG_WB: f32 = f32::from_bits(0x3d22_e000); // 0.039764404

/// Note → Hz (§3.3): `8.17742 · 2^(note/12)` with a cubic for the fractional octave.
const NOTE_K: f32 = f32::from_bits(0x3daa_a880); // 0.0833292
const P3: f32 = f32::from_bits(0x3d65_6042); // 0.056
const P2: f32 = f32::from_bits(0x3e77_ced9); // 0.242
const P1: f32 = f32::from_bits(0x3f31_6873); // 0.693
const HZ0: f32 = f32::from_bits(0x4102_d6b6); // 8.17742
const THIRD: f32 = f32::from_bits(0x3eaa_aa9f); // 0.333333
const FB_HZ: f32 = f32::from_bits(0x3ca3_d70a); // 0.02
const FB_BASE: f32 = f32::from_bits(0x3f6e_147b); // 0.93
const GAIN_MIN: f32 = f32::from_bits(0x3f34_fdf4); // 0.707
const FB_KNEE: f32 = f32::from_bits(0x3eb3_3333); // 0.35
const FB_SLOPE: f32 = f32::from_bits(0x3fc4_ec42); // 1.53846
const FB_MAX: f32 = f32::from_bits(0x3f33_3333); // 0.7
const OUT_GAIN: f32 = f32::from_bits(0x3f35_354c); // 0.7078445

/// The kernel's `round()`: add ±0.5 (by the sign of the `f32`) in `f64`, truncate.
fn rnd(x: f32) -> i32 {
    let h = if x < 0.0 { -0.5 } else { 0.5 };
    (h + x as f64) as i32
}

/// `2^n` built in the exponent bits, as the kernel does.
fn pow2(n: i32) -> f32 {
    f32::from_bits(0x3f80_0000u32.wrapping_add((n as u32) << 23))
}

/// The parabolic sine of a phase already wrapped to [−0.5, 0.5]: `v·(8 − 16|v|)`, which is
/// `sin(2πv)` to within 6 %.
fn para(v: f32) -> f32 {
    v * (8.0 - v.abs() * 16.0)
}

/// `2^x` for the smoothers' coefficients: a 1022-point table over one octave, linear
/// interpolation (the same routine and table as the Chorus kernel).
fn exp2_approx(x: f32) -> f32 {
    let n = {
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
    v * pow2(n)
}

/// Entry `i` of the octave table: `2^(i/1021 − 0.5)` to 6 significant digits, with the last
/// entry stored as √2 and seven near-ties rounded the other way.
fn exp2_entry(i: usize) -> f32 {
    match i {
        106 => return 0.759867,
        213 => return 0.817119,
        225 => return 0.823803,
        287 => return 0.859218,
        324 => return 0.881074,
        382 => return 0.91646,
        673 => return 1.11663,
        1022.. => return f32::from_bits(0x3fb5_04d5),
        _ => {}
    }
    let v = (i as f64 / 1021.0 - 0.5).exp2();
    let p = 10f64.powi(5 - v.log10().floor() as i32);
    ((v * p).round() / p) as f32
}

/// Six cascaded first-order all-passes `y = x[n−1] + c·(x − y[n−1])`. `s[0]` is the cascade's
/// previous input; `s[k]` is stage k's previous output plus `TINY`, which is also stage k+1's
/// previous input.
#[derive(Clone, Copy, Default)]
struct Cascade {
    c: [f32; 6],
    s: [f32; 7],
}

impl Cascade {
    fn run(&mut self, u: f32) -> f32 {
        let mut x = u;
        let mut prev = self.s[0];
        for k in 0..6 {
            let y = prev + self.c[k] * (x - self.s[k + 1]);
            prev = self.s[k + 1];
            self.s[k + 1] = y + TINY;
            x = y;
        }
        self.s[0] = u;
        x
    }
}

/// A smoothed parameter: target, state and coefficient (`f64`), and the `f32` value used.
#[derive(Clone, Copy, Default)]
struct Smoothed {
    t: f64,
    s: f64,
    c: f64,
    f: f32,
}

impl Smoothed {
    fn step(&mut self) {
        self.s += self.c * ((self.t - self.s) + TINY_D);
        self.f = self.s as f32;
    }

    /// `|f − t| ≤ 1e-4` (NaN counts as settled: the kernel's `b.gt` falls through).
    fn settled(&self) -> bool {
        let d = (self.f as f64 - self.t).abs();
        d <= EPS || d.is_nan()
    }
}

/// `FXCORE::FreqShifterCore`'s state (§3).
#[derive(Clone, Default)]
struct Core {
    sr: f32,
    /// Event inputs (normalized): Coarse, Fine, Feedback, Stereo, Invert (0/1), Mix.
    in_crs: f32,
    in_fine: f32,
    in_fb: f32,
    in_st: f32,
    in_inv: f32,
    in_mix: f32,
    /// Control-rate update flags (+0x5c); the block runs while any is set (§3.4).
    flags: u32,
    ctr: i32,
    period: i32,
    inv_sr: f32,
    /// Coarse note range: `lo + Coarse·range` (+0xb4/+0xb8), and the two note terms.
    note_lo: f32,
    note_range: f32,
    crs_note: f32,
    fine_st: f32,
    /// Feedback·0.7 (+0x274), its frequency limit (+0x270), and the output gain's frequency
    /// (+0x13c) and feedback (+0x140) factors.
    fb_scaled: f32,
    fb_lim: f32,
    gain_hz: f32,
    gain_fb: f32,
    freq: Smoothed,
    fb: Smoothed,
    gain: Smoothed,
    st: Smoothed,
    inv: Smoothed,
    mix: Smoothed,
    dry: f32,
    wet: f32,
    inc: f32,
    phase: f32,
    fb_l: f32,
    fb_r: f32,
    la: Cascade,
    lb: Cascade,
    ra: Cascade,
    rb: Cascade,
}

impl Core {
    /// `FreqShifterProcessor()`: a zeroed core with the processor's own input defaults.
    fn new(sr: f32) -> Core {
        Core {
            sr,
            in_crs: f32::from_bits(0x3f33_66c7),
            in_fb: f32::from_bits(0x3f47_ae14),
            in_st: f32::from_bits(0x3f63_d70a),
            in_inv: 1.0,
            in_mix: 1.0,
            ..Core::default()
        }
    }

    /// The shared tail of `processEventInCrs`/`processEventInFine` (§3.3): note → Hz, the
    /// feedback limit and the output gain.
    fn freq_event(&mut self) {
        let x = (self.crs_note + self.fine_st) * NOTE_K;
        let n = rnd(x);
        let f = x - n as f32;
        let p = f * (f * (f * P3 + P2) + P1);
        let hz = ((p + 1.0) * pow2(n)) * HZ0;
        let r = (1000.0 / hz) * 1.5;
        self.freq.t = hz as f64;
        self.fb_lim = ((hz * THIRD) * FB_HZ + FB_BASE).min(1.0);
        let g = if r < GAIN_MIN { GAIN_MIN } else { r };
        self.gain_hz = if r < 1.0 { g } else { 1.0 };
        self.fb.t = (self.fb_scaled * self.fb_lim) as f64;
        self.gain.t = ((self.gain_hz * self.gain_fb) * OUT_GAIN) as f64;
        self.flags |= 7;
    }

    fn crs_event(&mut self) {
        self.crs_note = self.note_lo + self.in_crs * self.note_range;
        self.freq_event();
    }

    fn fine_event(&mut self) {
        self.fine_st = self.in_fine * 12.0;
        self.freq_event();
    }

    /// `processEventInFB` (§3.3).
    fn fb_event(&mut self) {
        let v = self.in_fb;
        let a = (v + -FB_KNEE) * FB_SLOPE;
        let soft = (a * a) * -FB_KNEE + 1.0;
        self.fb_scaled = v * FB_MAX;
        self.fb.t = (self.fb_scaled * self.fb_lim) as f64;
        self.gain_fb = if v > FB_KNEE { soft } else { 1.0 };
        self.gain.t = ((self.gain_hz * self.gain_fb) * OUT_GAIN) as f64;
        self.flags |= 6;
    }

    fn st_event(&mut self) {
        let v = self.in_st;
        self.st.t = ((v * (v * v)) * 0.5 + 0.0) as f64;
        self.flags |= 8;
    }

    fn inv_event(&mut self) {
        self.inv.t = self.in_inv as f64;
        self.flags |= 0x10;
    }

    fn mix_event(&mut self) {
        self.mix.t = self.in_mix as f64;
        self.flags |= 0x20;
    }

    /// `FreqShifterProcessor::resume` + `FreqShifterCore::endInit` (§2, §3.2): clear
    /// everything, take the parameters as they are (no smoothing), and run one silent sample.
    fn resume(&mut self) {
        *self = Core {
            sr: self.sr,
            in_crs: self.in_crs,
            in_fine: self.in_fine,
            in_fb: self.in_fb,
            in_st: self.in_st,
            in_inv: self.in_inv,
            in_mix: self.in_mix,
            ..Core::default()
        };
        let sr = self.sr;

        // Control rate: one update every `steps` samples (f32 division here, unlike Chorus).
        let steps = rnd(sr / 9800.0);
        self.period = steps.wrapping_sub(1);
        let div = if (self.period as u32) > 0 { steps } else { 1 };
        let ctl = rnd(sr) / div;
        let per_ms = (ctl as f32) as f64 / 1000.0;

        // Smoothers: 100 ms (Mix, Feedback, gain, frequency) and 250 ms (Invert, Stereo).
        let log = ((LOG_A * LOG_WA + LOG_B * LOG_WB) + -61.0) as f64;
        let c100 = 1.0 - exp2_approx((log / (100.0 * per_ms + -1.0).max(1.0)) as f32) as f64;
        let c250 = 1.0 - exp2_approx((log / (250.0 * per_ms + -1.0).max(1.0)) as f32) as f64;
        self.mix.c = c100;
        self.fb.c = c100;
        self.gain.c = c100;
        self.freq.c = c100;
        self.inv.c = c250;
        self.st.c = c250;

        // Coarse spans the notes 69 + 12·(log entries − 8) … 69 + 12·(log entries + 2).
        let hi = ((HI_A * HI_WA + HI_B * HI_WB) + 2.0) * 12.0 + 69.0;
        self.note_lo = ((LO_A * LO_WA + LO_B * LO_WB) + -8.0) * 12.0 + 69.0;
        self.note_range = hi - self.note_lo;
        self.inv_sr = 1.0 / sr;

        // All-pass coefficients −(1 − w·k)/(1 + w·k), w = 15π/sr; both channels share them.
        let w = W_SCALE / sr;
        let coef = |k: u32| {
            let x = w * f32::from_bits(k);
            -(1.0 - x) / (x + 1.0)
        };
        let ca = K_A.map(coef);
        let cb = K_B.map(coef);
        self.la.c = ca;
        self.ra.c = ca;
        self.lb.c = cb;
        self.rb.c = cb;

        // The parameters, as their event handlers would set them; smoothers start at target.
        self.st_event();
        self.fine_st = self.in_fine * 12.0;
        self.fb_event();
        self.crs_event();
        self.mix_event();
        self.inv_event();
        for p in [&mut self.freq, &mut self.fb, &mut self.gain, &mut self.st, &mut self.inv, &mut self.mix] {
            p.s = p.t;
        }

        // endInit's own sample: silence in, output dropped. The control block runs once
        // (the counter starts at `steps`), so each smoother takes one step from its target.
        self.phase = -0.5;
        self.flags = 0xff;
        self.ctr = div;
        self.tick(0.0, 0.0);
    }

    /// `processEventInBfrE`, sent at the start of every block: clear the flag of each smoother
    /// whose `f32` value is within 1e-4 of its target. Bits 6–7 (two smoothers towards zero that
    /// feed nothing) always settle.
    fn bfr_e(&mut self) {
        let settled = [
            self.freq.settled(),
            self.fb.settled(),
            self.gain.settled(),
            self.st.settled(),
            self.inv.settled(),
            self.mix.settled(),
            true,
            true,
        ];
        for (bit, ok) in settled.into_iter().enumerate() {
            if ok {
                self.flags &= !(1 << bit);
            }
        }
    }

    /// The control-rate block of `processAudio` (§3.4).
    fn control(&mut self) {
        if self.ctr < self.period {
            self.ctr += 1;
            return;
        }
        self.ctr = 0;
        if (self.flags as i32) < 1 {
            return;
        }
        self.mix.step();
        let m = self.mix.f;
        self.wet = (2.0 - m) * m;
        let dm = 1.0 - m;
        self.dry = dm * (2.0 - dm);
        self.inv.step();
        self.st.step();
        self.fb.step();
        self.gain.step();
        self.freq.step();
        self.inc = self.inv_sr * self.freq.f;
    }

    /// `FreqShifterCore::processAudio` for one sample (§3.5).
    fn tick(&mut self, xl: f32, xr: f32) -> (f32, f32) {
        let ur = xr + self.fb_r;
        let ar = self.ra.run(ur);
        let br = self.rb.run(ur);
        let ul = xl + self.fb_l;
        let al = self.la.run(ul);
        let bl = self.lb.run(ul);
        self.control();

        // Quadrature oscillator: left at `phase`, right `Stereo³/2` cycles ahead.
        let p = self.phase + self.inc;
        let pl = p - rnd(p) as f32;
        let q = self.st.f + pl;
        self.phase = pl;
        let sin_r = para(q - rnd(q) as f32);
        let q4 = q + -0.25;
        let cos_r = para(q4 - rnd(q4) as f32);
        let inv = self.inv.f;
        let gain = self.gain.f;
        let fb = self.fb.f;

        // Invert 0 → B·sin + A·cos (shifts up), Invert 1 → B·sin − A·cos (shifts down); the
        // smoothed value crossfades between the two.
        let b = br * sin_r;
        let a = ar * cos_r;
        let sum = b + a;
        let out_r = sum + inv * ((b - a) - sum);
        let yr = xr * self.dry + self.wet * (gain * out_r);
        self.fb_r = fb * out_r + TINY;

        let sin_l = para(pl);
        let p4 = pl + -0.25;
        let cos_l = para(p4 - rnd(p4) as f32);
        let b = bl * sin_l;
        let a = al * cos_l;
        let sum = b + a;
        let out_l = sum + inv * ((b - a) - sum);
        let yl = xl * self.dry + self.wet * (gain * out_l);
        self.fb_l = fb * out_l + TINY;
        (yl, yr)
    }
}

/// The Freq Shifter effect: the parameters (`coarse`, `fine`, `feedback`, `stereo`,
/// `invert`, `mix`) go to the core as events.
pub struct FreqShifter {
    core: Core,
    started: bool,
}

impl FreqShifter {
    pub fn new(sr: f64) -> Self {
        FreqShifter { core: Core::new(sr as f32), started: false }
    }
}

impl Effect for FreqShifter {
    fn set(&mut self, params: &[(String, f64)]) {
        let crs = param(params, "coarse", 0.5) as f32;
        let fine = param(params, "fine", 0.0) as f32;
        let fb = param(params, "feedback", 0.75) as f32;
        let st = param(params, "stereo", 0.75) as f32;
        let inv = if param(params, "invert", 1.0) != 0.0 { 1.0 } else { 0.0 };
        let mix = param(params, "mix", 0.75) as f32;
        let c = &mut self.core;
        if !self.started {
            // The processor resumes with the stored values (§2).
            self.started = true;
            (c.in_crs, c.in_fine, c.in_fb, c.in_st, c.in_inv, c.in_mix) = (crs, fine, fb, st, inv, mix);
            c.resume();
            return;
        }
        // Later changes arrive as events (new smoother targets); unchanged values send none.
        if fb != c.in_fb {
            c.in_fb = fb;
            c.fb_event();
        }
        if crs != c.in_crs {
            c.in_crs = crs;
            c.crs_event();
        }
        if fine != c.in_fine {
            c.in_fine = fine;
            c.fine_event();
        }
        if st != c.in_st {
            c.in_st = st;
            c.st_event();
        }
        if inv != c.in_inv {
            c.in_inv = inv;
            c.inv_event();
        }
        if mix != c.in_mix {
            c.in_mix = mix;
            c.mix_event();
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.core.bfr_e();
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            (*a, *b) = self.core.tick(*a, *b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(crs: f64, fine: f64, fb: f64, st: f64, inv: f64, mix: f64) -> Vec<(String, f64)> {
        [("coarse", crs), ("fine", fine), ("feedback", fb), ("stereo", st), ("invert", inv), ("mix", mix)]
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect()
    }

    fn noise(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i * 7919 + 13) % 1000) as f32 / 1000.0 - 0.5).collect()
    }

    fn sine(hz: f32, sr: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (std::f32::consts::TAU * hz * i as f32 / sr).sin() * 0.5).collect()
    }

    fn run(fx: &mut FreqShifter, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut a, mut b) = (l.to_vec(), r.to_vec());
        for (x, y) in a.chunks_mut(512).zip(b.chunks_mut(512)) {
            fx.process(x, y);
        }
        (a, b)
    }

    /// Goertzel power at `hz`.
    fn power(x: &[f32], hz: f32, sr: f32) -> f32 {
        let w = std::f32::consts::TAU * hz / sr;
        let k = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for v in x {
            let s = v + k * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        s1 * s1 + s2 * s2 - k * s1 * s2
    }

    #[test]
    fn coarse_spans_three_hz_to_two_khz_and_fine_an_octave() {
        let mut c = Core::new(44_100.0);
        c.in_crs = 0.0;
        c.resume();
        assert!((c.freq.t - 3.0).abs() < 0.1, "{}", c.freq.t);
        c.in_crs = 1.0;
        c.resume();
        assert!((c.freq.t - 2000.0).abs() < 50.0, "{}", c.freq.t);
        c.in_fine = 1.0;
        c.resume();
        assert!((c.freq.t - 4000.0).abs() < 100.0, "{}", c.freq.t);
    }

    #[test]
    fn shifts_a_sine_up_or_down() {
        let sr = 44_100.0;
        let x = sine(1000.0, sr, 44_100);
        let mut c = Core::new(sr);
        c.resume();
        // Coarse for a 100 Hz shift with Fine 0.
        let crs = ((100f32 / 8.17742).log2() * 12.0 - c.note_lo) / c.note_range;
        let mut seen = vec![];
        for inv in [0.0, 1.0] {
            let mut fx = FreqShifter::new(sr as f64);
            fx.set(&p(crs as f64, 0.0, 0.0, 0.0, inv, 1.0));
            let (l, _) = run(&mut fx, &x, &x);
            let hz = fx.core.freq.t as f32;
            let tail = &l[8000..];
            let (up, down) = (power(tail, 1000.0 + hz, sr), power(tail, 1000.0 - hz, sr));
            assert!(up > 100.0 * down || down > 100.0 * up, "inv {inv}: {up} {down}");
            assert!(up.max(down) > 100.0 * power(tail, 1000.0, sr), "inv {inv}");
            seen.push(up > down);
        }
        // The two Invert settings shift in opposite directions.
        assert_ne!(seen[0], seen[1]);
    }

    #[test]
    fn dry_mix_is_the_identity() {
        let mut fx = FreqShifter::new(48_000.0);
        fx.set(&p(0.7, 0.3, 0.9, 1.0, 1.0, 0.0));
        let x = noise(20_000);
        let (l, r) = run(&mut fx, &x, &x);
        assert_eq!(l, x);
        assert_eq!(r, x);
    }

    #[test]
    fn channels_are_independent_and_silence_stays_silent() {
        let mut fx = FreqShifter::new(44_100.0);
        fx.set(&p(0.6, 0.0, 0.9, 0.5, 1.0, 1.0));
        let x = noise(30_000);
        let z = vec![0.0; x.len()];
        let (l, r) = run(&mut fx, &x, &z);
        assert!(l.iter().any(|v| v.abs() > 0.01));
        // Only the kernel's 1e-18 state offsets leak through on the silent side.
        assert!(r.iter().all(|v| v.abs() < 1e-13));
    }

    #[test]
    fn set_keeps_state_and_smooths() {
        let x = noise(40_000);
        let mut a = FreqShifter::new(44_100.0);
        a.set(&p(0.4, 0.2, 0.5, 0.3, 1.0, 0.75));
        let (l1, _) = run(&mut a, &x[..10_240], &x[..10_240]);
        a.set(&p(0.4, 0.2, 0.5, 0.3, 1.0, 0.75));
        let (l2, _) = run(&mut a, &x[10_240..], &x[10_240..]);
        let mut b = FreqShifter::new(44_100.0);
        b.set(&p(0.4, 0.2, 0.5, 0.3, 1.0, 0.75));
        let (lb, _) = run(&mut b, &x, &x);
        assert_eq!([l1, l2].concat(), lb);

        // Mix 0 → 1 glides over several control steps, then settles and freezes.
        let mut c = FreqShifter::new(44_100.0);
        c.set(&p(0.4, 0.2, 0.5, 0.3, 1.0, 0.0));
        run(&mut c, &x[..1024], &x[..1024]);
        c.set(&p(0.4, 0.2, 0.5, 0.3, 1.0, 1.0));
        run(&mut c, &x[..64], &x[..64]);
        assert!(c.core.dry > 0.1 && c.core.dry < 1.0, "{}", c.core.dry);
        run(&mut c, &x, &x);
        assert!(c.core.dry < 1e-3, "{}", c.core.dry);
        assert_eq!(c.core.flags, 0);
    }

    #[test]
    fn full_feedback_stays_bounded() {
        for sr in [44_100.0, 96_000.0] {
            for crs in [0.0, 0.5, 1.0] {
                let mut fx = FreqShifter::new(sr);
                fx.set(&p(crs, 1.0, 1.0, 1.0, 0.0, 1.0));
                let x: Vec<f32> = noise(100_000).iter().map(|v| v * 4.0).collect();
                let (l, r) = run(&mut fx, &x, &x);
                assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 1000.0), "{sr} {crs}");
            }
        }
    }
}
