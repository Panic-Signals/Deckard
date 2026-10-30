//! Maschine's Gate (docs/fx-gate.md): `RT::GateProcessor` → `FXCORE::GateCore`, a compiled
//! Reaktor-Core kernel. A peak detector (on the side-chain, when it has a source) re-arms a
//! hold counter whenever it exceeds the threshold; while the counter is inside the hold
//! time the envelope rises toward 1 at the attack rate, otherwise it falls toward 0 at the
//! release rate. The gain is `1 + mix·(env − 1)`, with Mix smoothed over ≈ 100 ms. All
//! arithmetic follows the assembly operation by operation (`f32`, no fused multiply-adds),
//! so the output can match bit for bit.

use super::sidechain::SidechainFilter;
use super::{Effect, param};

/// An `f32` constant as Maschine stores it.
const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}
/// The smoothers' denormal offset (`0x219392ef`, 1e-18).
const TINY: f32 = c(0x2193_92ef);
/// The hysteresis: the gate re-triggers above `threshold² − 0.01` (§4.1).
const HYST: f32 = c(0x3c23_d892);
/// The hold lasts at least `35 ms − release` (§4.2).
const MIN_HOLD_MS: f32 = 35.0;

/// Field names and defaults (stored values) of `DATA::Gate`, in `tFloatParameters` order.
const PARAMS: [(&str, u32); 5] = [
    ("threshold", 0x3f0f_f2e5), // 0.5623 (−10 dB)
    ("attack", 0x0000_0000),
    ("hold", 0x3f00_0000),
    ("release", 0x3e80_0000),
    ("mix", 0x3f80_0000),
];

/// `round()` as the kernel does it: add ±0.5 (by the sign of the `f32`) in `f64`, truncate.
fn round_away(x: f32) -> i32 {
    let h = if x < 0.0 { -0.5 } else { 0.5 };
    (h + x as f64) as i32
}

/// `2^x` (§3.1): a 1022-point table over one octave, linear interpolation, and the
/// integer part put into the exponent bits. The same kernel table as the Chorus's.
pub(crate) fn exp2_approx(x: f32) -> f32 {
    let n = round_away(x);
    let y = ((x - n as f32) + 0.5) * 1021.0;
    let k = round_away(y + -0.5);
    let fr = y - k as f32;
    let i = (k as u32 & 0x3ff) as usize;
    let v = (1.0 - fr) * exp2_entry(i) + fr * exp2_entry(i + 1);
    v * f32::from_bits(0x3f80_0000u32.wrapping_add((n as u32) << 23))
}

/// Entry `i` of the kernel's octave table: `2^(i/1021 − 0.5)` to 6 significant digits,
/// with the last entry stored as √2 and seven near-ties rounded the other way (checked
/// against all 1023 stored entries).
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

/// `log2(0.01)` and `log2(0.1)` as the kernel gets them (§3.2): two entries of a log table,
/// interpolated at a constant position, plus the octave.
fn log2_consts() -> (f32, f32) {
    let l001 = (c(0x3eb5_d91b) * c(0x3df5_d000) + c(0x3eb6_69cf) * c(0x3f61_4600)) + -7.0;
    let l01 = (c(0x3f2d_7364) * c(0x3ecc_c800) + c(0x3f2d_ad43) * c(0x3f19_9c00)) + -4.0;
    (l001, l01)
}

/// A one-pole coefficient that falls to −20 dB in `n` steps: `1 − 0.1^(1/max(n, 1))`.
fn coef(l01: f32, n: f32) -> f32 {
    let n = if n > 1.0 { n } else { 1.0 }; // fmaxnm
    1.0 - exp2_approx(l01 / n)
}

/// `FXCORE::GateCore` (its fields with their object offsets; §3).
struct Core {
    /// Sample rate (+0x00) and samples per millisecond (+0x84 = +0x9c = +0xa8).
    sr: f32,
    k: f32,
    /// The detector takes the side-chain (+0x14).
    sc: bool,
    // Inputs as the processor last sent them: threshold² (+0x28), Attack, Hold, Release
    // (+0x30/+0x38/+0x40, stored values) and Mix (+0x1c).
    thr: f32,
    att_v: f32,
    hold_v: f32,
    rel_v: f32,
    mix_v: f32,
    /// `log2(0.01)`, `log2(0.1)` (+0x6c; +0x70 = +0x98 = +0xa4).
    l001: f32,
    l01: f32,
    /// Mix: target `v(2 − v)` (+0x58), smoothed (+0x5c), smoothing coefficient (+0x60).
    mix_t: f32,
    mix_s: f32,
    mix_c: f32,
    /// The trigger level `threshold² − 0.01` (+0x68).
    thr_eff: f32,
    /// The attack's effective length (ms, +0x74), Hold (ms, +0x78), the effective hold
    /// (ms, +0x7c), Release (ms, +0x80).
    att_eff: f32,
    hold_ms: f32,
    hold_eff: f32,
    rel_ms: f32,
    /// The hold in samples (+0x88), latched at the last trigger (+0x8c), and the samples
    /// since that trigger (+0x90).
    hold_n: i32,
    latch: i32,
    count: i32,
    /// The envelope (+0x94) and its attack and release coefficients (+0xa0, +0xac).
    env: f32,
    att_c: f32,
    rel_c: f32,
}

impl Core {
    fn new(sr: f32) -> Core {
        Core {
            sr,
            k: 0.0,
            sc: false,
            thr: 0.0,
            att_v: 0.0,
            hold_v: 0.0,
            rel_v: 0.0,
            mix_v: 0.0,
            l001: 0.0,
            l01: 0.0,
            mix_t: 0.0,
            mix_s: 0.0,
            mix_c: 0.0,
            thr_eff: 0.0,
            att_eff: 0.0,
            hold_ms: 0.0,
            hold_eff: 0.0,
            rel_ms: 0.0,
            hold_n: 0,
            latch: 0,
            count: 0,
            env: 0.0,
            att_c: 0.0,
            rel_c: 0.0,
        }
    }

    /// `hold_n = round(k·(hold_eff + max(35 − (release + hold_eff), 0)))`.
    fn hold_samples(&mut self) {
        let x = MIN_HOLD_MS - (self.rel_ms + self.hold_eff);
        let extra = if x > 0.0 { x } else { 0.0 }; // fmaxnm with +0
        self.hold_n = round_away(self.k * (self.hold_eff + extra));
    }

    /// The attack's length (ms) and its effective length (§4.2).
    fn attack_times(&self) -> (f32, f32) {
        let a = self.att_v * self.att_v * c(0x4379_c000) + 0.25; // 0.25..250 ms
        let m = a + -1.0;
        let m = if m > 1.0 { m } else { 1.0 };
        let r = (1.0 - exp2_approx(self.l001 / m)) / (1.0 - exp2_approx(self.l01 / m));
        (a, a * r)
    }

    fn max_hold(&mut self) {
        self.hold_eff = if self.att_eff > self.hold_ms { self.att_eff } else { self.hold_ms };
    }

    // `GateProcessor::processFloatParameter` (§2.2).

    fn set_threshold(&mut self, v: f32) {
        self.thr = v * v;
        self.thr_eff = self.thr - HYST;
    }

    /// Attack: `processEventInAtt`.
    fn set_attack(&mut self, v: f32) {
        self.att_v = v;
        let (a, eff) = self.attack_times();
        self.att_eff = eff;
        self.max_hold();
        self.hold_samples();
        self.att_c = coef(self.l01, a * self.k + -1.0);
    }

    fn set_hold(&mut self, v: f32) {
        self.hold_v = v;
        self.hold_ms = v * v * c(0x43f8_c000) + 2.5; // 2.5..500 ms
        self.max_hold();
        self.hold_samples();
    }

    fn set_release(&mut self, v: f32) {
        self.rel_v = v;
        self.rel_ms = v * v * c(0x43f6_4000) + 7.5; // 7.5..500 ms
        self.hold_samples();
        self.rel_c = coef(self.l01, self.rel_ms * self.k + -1.0);
    }

    fn set_mix(&mut self, v: f32) {
        self.mix_v = v;
        self.mix_t = (2.0 - v) * v;
    }

    /// `GateProcessor::resume`: the state clears, then `endInit` derives every coefficient
    /// from the inputs and runs one silent sample.
    fn resume(&mut self) {
        let (l001, l01) = log2_consts();
        self.l001 = l001;
        self.l01 = l01;
        self.k = self.sr / 1000.0;
        self.thr_eff = self.thr + -HYST;
        self.hold_ms = self.hold_v * self.hold_v * c(0x43f8_c000) + 2.5;
        self.rel_ms = self.rel_v * self.rel_v * c(0x43f6_4000) + 7.5;
        self.mix_t = (2.0 - self.mix_v) * self.mix_v;
        let (a, eff) = self.attack_times();
        self.att_eff = eff;
        self.max_hold();
        self.mix_c = coef(l01, self.k * 100.0 + -1.0);
        self.rel_c = coef(l01, self.k * self.rel_ms + -1.0);
        self.mix_s = self.mix_t;
        self.hold_samples();
        self.att_c = coef(l01, self.k * a + -1.0);
        self.env = 0.0;
        self.latch = 0;
        self.count = 0;
        self.step(0.0, 0.0, 0.0);
    }

    /// One sample of the kernel on input `(l, r)` with detector level `d`.
    #[inline]
    fn step(&mut self, d: f32, l: f32, r: f32) -> (f32, f32) {
        self.mix_s += self.mix_c * ((self.mix_t - self.mix_s) + TINY);
        if d > self.thr_eff {
            self.count = 0;
            self.latch = self.hold_n;
        }
        let target = if self.latch > self.count { 1.0 } else { 0.0 };
        let diff = target - self.env;
        let k = if diff > 0.0 { self.att_c } else { self.rel_c };
        self.env += k * (diff + TINY);
        let g = self.mix_s * (self.env + -1.0) + 1.0;
        self.count = self.count.wrapping_add(1);
        (l * g, r * g)
    }

    fn gain(&self) -> f32 {
        self.mix_s * (self.env + -1.0) + 1.0
    }
}

pub struct Gate {
    core: Core,
    values: Option<[f32; 5]>,
    scf: SidechainFilter,
    /// The filtered side-chain block.
    sc: [Vec<f32>; 2],
}

impl Gate {
    pub fn new(sr: f64) -> Self {
        Gate { core: Core::new(sr as f32), values: None, scf: SidechainFilter::new(sr), sc: [Vec::new(), Vec::new()] }
    }

    fn apply(&mut self, k: usize, v: f32) {
        let c = &mut self.core;
        match k {
            0 => c.set_threshold(v),
            1 => c.set_attack(v),
            2 => c.set_hold(v),
            3 => c.set_release(v),
            _ => c.set_mix(v),
        }
    }
}

impl Effect for Gate {
    fn set(&mut self, params: &[(String, f64)]) {
        let new = PARAMS.map(|(name, def)| param(params, name, c(def) as f64) as f32);
        match self.values {
            None => {
                // The first settings reach the processor before it resumes.
                for (k, v) in new.iter().enumerate() {
                    self.apply(k, *v);
                }
                self.core.resume();
            }
            Some(old) => {
                for k in 0..5 {
                    if new[k].to_bits() != old[k].to_bits() {
                        self.apply(k, new[k]);
                    }
                }
            }
        }
        self.values = Some(new);
        self.scf.set(params);
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.process_sc(l, r, None)
    }

    /// Its panel shows the input level on the Threshold fader.
    fn metered(&self) -> bool {
        true
    }

    /// The gain the gate applies now (1 = open).
    fn gain_reduction(&self) -> f32 {
        self.core.gain()
    }

    /// `GateProcessor::process`: with a side-chain, the side-chain filter runs over it
    /// first, and the detector takes it instead of the input.
    fn process_sc(&mut self, l: &mut [f32], r: &mut [f32], sc: Option<(&[f32], &[f32])>) {
        let n = l.len().min(r.len());
        self.core.sc = sc.is_some();
        let [sl, sr] = &mut self.sc;
        if let Some((a, b)) = sc {
            sl.clear();
            sl.extend_from_slice(&a[..n]);
            sr.clear();
            sr.extend_from_slice(&b[..n]);
            self.scf.process(sl, sr);
        }
        for i in 0..n {
            let (xl, xr) = (l[i], r[i]);
            let (dl, dr) = if self.core.sc { (sl[i].abs(), sr[i].abs()) } else { (xl.abs(), xr.abs()) };
            let d = if dl > dr { dl } else { dr };
            (l[i], r[i]) = self.core.step(d, xl, xr);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(threshold: f64, attack: f64, hold: f64, release: f64, mix: f64) -> Vec<(String, f64)> {
        [("threshold", threshold), ("attack", attack), ("hold", hold), ("release", release), ("mix", mix)]
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect()
    }

    fn run(fx: &mut Gate, x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (x.to_vec(), x.iter().map(|v| v * 0.5).collect::<Vec<_>>());
        for (a, b) in l.chunks_mut(300).zip(r.chunks_mut(300)) {
            fx.process(a, b);
        }
        (l, r)
    }

    /// A sine at `amp` over `on`, at −60 dB elsewhere.
    fn burst(n: usize, on: std::ops::Range<usize>, amp: f32) -> Vec<f32> {
        (0..n).map(|i| (i as f32 * 0.0573).sin() * if on.contains(&i) { amp } else { 0.001 }).collect()
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0f32, |m, v| m.max(v.abs()))
    }

    #[test]
    fn log2_constants() {
        let (a, b) = log2_consts();
        assert!((a - 0.01f32.log2()).abs() < 1e-3, "{a}");
        assert!((b - 0.1f32.log2()).abs() < 1e-3, "{b}");
    }

    #[test]
    fn exp2_approx_is_two_to_the_x() {
        for i in -400..400 {
            let x = i as f32 * 0.037;
            assert!((exp2_approx(x) / x.exp2() - 1.0).abs() < 1e-5, "{x}");
        }
    }

    #[test]
    fn opens_above_and_closes_below_the_threshold() {
        let mut fx = Gate::new(48_000.0);
        fx.set(&params(0.5623, 0.0, 0.0, 0.0, 1.0)); // −10 dB
        let x = burst(48_000, 4800..24_000, 0.8);
        let (l, r) = run(&mut fx, &x);
        // Open, the burst passes unchanged; the tail is gated away within ~50 ms.
        assert!((peak(&l[12_000..24_000]) - 0.8).abs() < 1e-3);
        assert!(peak(&l[30_000..]) < 1e-6, "{}", peak(&l[30_000..]));
        assert!(peak(&l[..4000]) < 1e-6);
        // Both channels take the same gain.
        for i in 0..x.len() {
            assert!((r[i] - l[i] * 0.5).abs() <= 1e-6 * l[i].abs().max(1e-6), "{i}");
        }
        assert!(fx.gain_reduction() < 1e-6);
    }

    #[test]
    fn mix_zero_is_transparent() {
        let mut fx = Gate::new(44_100.0);
        fx.set(&params(1.0, 0.3, 0.3, 0.3, 0.0));
        let x = burst(9000, 0..100, 0.5);
        let (l, _) = run(&mut fx, &x);
        assert_eq!(l, x);
    }

    #[test]
    fn mix_sets_the_floor() {
        // Mix 0.5 → v(2 − v) = 0.75 of the way: the closed gate leaves 25 %.
        let mut fx = Gate::new(48_000.0);
        fx.set(&params(1.0, 0.0, 0.0, 0.0, 0.5));
        let (l, _) = run(&mut fx, &[0.25f32; 48_000]);
        assert!((l[47_999] - 0.0625).abs() < 1e-5, "{}", l[47_999]);
    }

    #[test]
    fn hold_keeps_it_open() {
        // Hold 1.0 = 500 ms: after a short burst the gate stays open about that long.
        let mut fx = Gate::new(48_000.0);
        fx.set(&params(0.5623, 0.0, 1.0, 0.0, 1.0));
        let x = burst(48_000, 0..2400, 0.8);
        let (l, _) = run(&mut fx, &x);
        let open = |i: usize| peak(&l[i..i + 500]) > 0.0009;
        assert!(open(20_000), "still open 400 ms in");
        assert!(!open(30_000), "closed after the hold and release");
    }

    #[test]
    fn side_chain_drives_the_detector() {
        let mut fx = Gate::new(48_000.0);
        let mut p = params(0.5623, 0.0, 0.0, 0.0, 1.0);
        p.push(("side_chain_filter/gain".into(), 0.5)); // 0 dB
        fx.set(&p);
        let quiet = vec![0.0f32; 9600];
        let (mut l, mut r) = (vec![0.9f32; 9600], vec![0.9f32; 9600]);
        fx.process_sc(&mut l, &mut r, Some((&quiet, &quiet)));
        assert!(peak(&l[4800..]) < 1e-6, "a silent side-chain closes the gate");
        let loud = vec![0.9f32; 9600];
        let (mut l, mut r) = (vec![0.1f32; 9600], vec![0.1f32; 9600]);
        fx.process_sc(&mut l, &mut r, Some((&loud, &loud)));
        assert!((l[9000] - 0.1).abs() < 1e-6, "a loud one opens it");
    }

    #[test]
    fn parameter_changes_keep_state() {
        let x = burst(9600, 0..9600, 0.5);
        let mut fx = Gate::new(48_000.0);
        fx.set(&params(0.5623, 0.2, 0.5, 0.25, 1.0));
        run(&mut fx, &x);
        let env = fx.core.env;
        fx.set(&params(0.5623, 0.2, 0.5, 0.9, 1.0));
        assert_eq!(fx.core.env.to_bits(), env.to_bits());
        let (l, _) = run(&mut fx, &x);
        assert!(l.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn silence_stays_finite() {
        let mut fx = Gate::new(44_100.0);
        fx.set(&params(0.1, 1.0, 1.0, 1.0, 1.0));
        let (l, _) = run(&mut fx, &[0.0f32; 441_000]);
        assert!(l.iter().all(|v| v.is_finite()));
        assert!(fx.core.env.is_finite() && fx.core.mix_s.is_finite());
    }
}
