//! Maschine's Maximizer (docs/fx-maximizer.md): `FXCORE::MaximizerCore`, a per-sample
//! peak-envelope gain rider with a hold, a curve between square-root and full gain
//! reduction, and a tanh output clipper. Turbo adds a second rider stage in front. With a
//! side-chain source, the side-chain filter shapes it and the detector sees it.

use super::compressor::e;
use super::sidechain::SidechainFilter;
use super::{Effect, param};

/// An `f32` constant as Maschine stores it.
const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}
const K_LN: f32 = c(0x42ad_1f97); // 60 / ln 2: natural log → table index
const LN2: f32 = c(0x3f31_7215);
const TINY: f32 = c(0x0da2_4260); // 1e-30
const G_MAX: f32 = c(0x461c_4000); // 10000
const A_MIN: f32 = c(0x3727_c5ac); // 1e-5
const AMT_K: f32 = c(0xbf7d_70a4); // −0.99
const ATK: f32 = c(0x44bd_64d9); // 1515.1515 = 1 / 0.66 ms
const REL: f32 = c(0x40f6_2762); // 7.6923 = 1 / 130 ms
const RAMP_T: f32 = c(0x3ca3_d70a); // 0.02 s

/// `1/x` per lane as NEON `frecpe` and two `frecps` Newton steps.
fn recip(x: [f32; 4]) -> [f32; 4] {
    #[cfg(target_arch = "aarch64")]
    {
        use std::arch::aarch64::*;
        // SAFETY: NEON is always present on aarch64; the arrays are 4 floats.
        unsafe {
            let v = vld1q_f32(x.as_ptr());
            let r = vrecpeq_f32(v);
            let r = vmulq_f32(r, vrecpsq_f32(v, r));
            let r = vmulq_f32(vrecpsq_f32(v, r), r);
            let mut o = [0f32; 4];
            vst1q_f32(o.as_mut_ptr(), r);
            o
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        x.map(|v| 1.0 / v)
    }
}

/// `√x` per lane as `x·r·(3 − x·r²)/2`, `r` from `frsqrte` and one `frsqrts` step, and 0
/// where `x == 0` (§3.4).
fn sqrt(x: [f32; 4]) -> [f32; 4] {
    #[cfg(target_arch = "aarch64")]
    {
        use std::arch::aarch64::*;
        // SAFETY: NEON is always present on aarch64; the arrays are 4 floats.
        unsafe {
            let v = vld1q_f32(x.as_ptr());
            let r0 = vrsqrteq_f32(v);
            let r1 = vmulq_f32(r0, vrsqrtsq_f32(vmulq_f32(v, r0), r0));
            let t = vmulq_f32(v, r1);
            let s = vmulq_f32(vrsqrtsq_f32(t, r1), t);
            let z = vceqzq_f32(v);
            let s = vreinterpretq_f32_u32(vbicq_u32(vreinterpretq_u32_f32(s), z));
            let mut o = [0f32; 4];
            vst1q_f32(o.as_mut_ptr(), s);
            o
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        x.map(f32::sqrt)
    }
}

/// `k = c/sr` as a one-pole coefficient: `ln2·k`, or `ln2` when `k > 1` (`beginInit`).
fn coef(k: f32) -> f32 {
    if k > 1.0 { LN2 } else { k * LN2 }
}

/// The follower coefficients (§3.1).
struct Coefs {
    atk: f32,
    rel: f32,
    /// The hold's decay per sample: from 1 to 0.5 takes ≈ 10 ms.
    hold: f32,
}

/// One peak follower with a hold (§3.3).
#[derive(Clone, Copy)]
struct Env {
    env: f32,
    hold: f32,
}

const ENV0: Env = Env { env: 0.0, hold: 1.0 };

impl Env {
    /// Follow peak `d`; returns the gain divisor `min(max(env, a), 10000)`.
    fn step(&mut self, d: f32, a: f32, k: &Coefs) -> f32 {
        let diff = d - self.env;
        let q = if diff > 0.0 {
            self.hold = 1.0;
            k.atk
        } else if self.hold > 0.5 {
            self.hold *= 1.0 - k.hold;
            0.0
        } else {
            k.rel
        };
        let y = diff.mul_add(q, self.env);
        self.env = y + TINY;
        let g = if y < a { a } else { y };
        g.min(G_MAX)
    }
}

/// The stereo-linked peak `max(|l|, |r|)` (as `|l| < |r| ? |r| : |l|`).
fn peak(l: f32, r: f32) -> f32 {
    let (l, r) = (l.abs(), r.abs());
    if l < r { r } else { l }
}

/// `FXCORE::MaximizerCore` (§3).
struct Core {
    k: Coefs,
    /// Parameter ramps (§3.2): lane 0 = threshold `a`, lane 1 = Curve.
    cur: [f32; 2],
    inc: [f32; 2],
    tgt: [f32; 2],
    dirty: bool,
    countdown: i32,
    ticks: i32,
    inv: f32,
    counter: i32,
    turbo: bool,
    env1: Env,
    env2: Env,
    prev_in: [f32; 2],
    prev_out: [f32; 2],
}

impl Core {
    fn new(sr: f32) -> Core {
        let isr = 1.0 / sr;
        let ticks = (((sr * RAMP_T) / 128.0 + 0.5) as i32).max(1);
        Core {
            k: Coefs { atk: coef(ATK * isr), rel: coef(REL * isr), hold: coef(isr * 100.0) },
            cur: [0.0; 2],
            inc: [0.0; 2],
            tgt: [0.0; 2],
            dirty: false,
            countdown: 0,
            ticks,
            inv: 1.0 / (ticks * 128) as f32,
            counter: 129,
            turbo: false,
            env1: ENV0,
            env2: ENV0,
            prev_in: [0.0; 2],
            prev_out: [0.0; 2],
        }
    }

    /// `processEventInAmt`: lane 0 ← `max((1 − 0.99·amount)², 1e-5)`.
    fn set_amount(&mut self, v: f32) {
        let a = v.mul_add(AMT_K, 1.0);
        self.tgt[0] = (a * a).max(A_MIN);
        self.dirty = true;
    }

    /// `processEventInCrv`.
    fn set_curve(&mut self, v: f32) {
        self.tgt[1] = v;
        self.dirty = true;
    }

    /// `processEventInTurbo`: switching on restarts the first stage.
    fn set_turbo(&mut self, on: bool) {
        self.turbo = on;
        if on {
            self.env2 = Env { env: 1.0, hold: 1.0 };
            self.prev_out = self.prev_in;
        }
    }

    /// `beginInit` + `endInit` (from `MaximizerProcessor::resume`): the state clears and
    /// the parameters apply at once.
    fn resume(&mut self) {
        self.cur = self.tgt;
        self.inc = [0.0; 2];
        self.dirty = true;
        self.countdown = 0;
        self.counter = 129;
        self.env1 = ENV0;
        self.env2 = ENV0;
        self.prev_in = [0.0; 2];
        self.prev_out = [0.0; 2];
        self.set_turbo(self.turbo);
    }

    /// A control tick of the ramps.
    fn tick(&mut self) {
        if self.dirty {
            self.dirty = false;
            let mut any = false;
            for k in 0..2 {
                self.inc[k] = (self.tgt[k] - self.cur[k]) * self.inv;
                if ((self.inc[k] * self.inc[k]) as f64) < 1e-15 {
                    self.inc[k] = 0.0;
                    self.cur[k] = self.tgt[k];
                } else {
                    any = true;
                }
            }
            if any {
                self.countdown = self.ticks;
            }
        } else {
            self.countdown = self.countdown.wrapping_sub(1);
            if self.countdown == 0 {
                self.cur = self.tgt;
                self.inc = [0.0; 2];
            }
        }
    }

    /// One sample (`processAudio`) of input `x` and side-chain `s` (if any); the output
    /// is one sample late (with or without Turbo).
    fn process(&mut self, (xl, xr): (f32, f32), s: Option<(f32, f32)>) -> (f32, f32) {
        self.cur[0] += self.inc[0];
        self.cur[1] += self.inc[1];
        self.counter += 1;
        if self.counter >= 128 {
            self.counter = 0;
            self.tick();
        }
        let [a, crv] = self.cur;

        // Turbo's stage rides the input; the main stage rides its output (or the input).
        let (g2, [u, v]) = if self.turbo {
            let (dl, dr) = s.unwrap_or((xl, xr));
            (self.env2.step(peak(dl, dr), a, &self.k), self.prev_out)
        } else {
            (1.0, self.prev_in)
        };
        let d1 = match s {
            Some((sl, sr)) if !self.turbo => peak(sl, sr),
            _ => peak(u, v),
        };
        let g1 = self.env1.step(d1, a, &self.k);

        // Gain: from √(1/g) (Curve 0) to 1/g (Curve 1).
        let r = recip([g2, g2, g1, g1]);
        let q = sqrt(r);
        let x = [xl, xr, u, v];
        let y: [f32; 4] = std::array::from_fn(|k| x[k] * (q[k] + (r[k] - q[k]) * crv));
        self.prev_out = [y[0], y[1]];
        self.prev_in = [xl, xr];

        // tanh(y) = (e^2y − 1)/(e^2y + 1), e^2y from the exp table.
        let el = e((y[2] + y[2]).mul_add(K_LN, 1200.0));
        let er = e((y[3] + y[3]).mul_add(K_LN, 1200.0));
        let ri = recip([el + 1.0, er + 1.0, 2.0, 2.0]);
        ((el + -1.0) * ri[0], (er + -1.0) * ri[1])
    }
}

/// Field names and defaults: Amount, Curve, Turbo.
const PARAMS: [(&str, f32); 3] = [("amount", 0.5), ("curve", 0.5), ("turbo", 0.0)];

pub struct Maximizer {
    core: Core,
    values: Option<[f32; 3]>,
    scf: SidechainFilter,
    /// The filtered side-chain block.
    sc: [Vec<f32>; 2],
}

impl Maximizer {
    pub fn new(sr: f64) -> Maximizer {
        Maximizer { core: Core::new(sr as f32), values: None, scf: SidechainFilter::new(sr), sc: [Vec::new(), Vec::new()] }
    }

    fn apply(&mut self, k: usize, v: f32) {
        match k {
            0 => self.core.set_amount(v),
            1 => self.core.set_curve(v),
            _ => self.core.set_turbo(v != 0.0),
        }
    }
}

impl Effect for Maximizer {
    fn set(&mut self, params: &[(String, f64)]) {
        let new = PARAMS.map(|(name, def)| param(params, name, def as f64) as f32);
        match self.values {
            None => {
                // The first settings reach the core before the processor resumes.
                for (k, v) in new.iter().enumerate() {
                    self.apply(k, *v);
                }
                self.core.resume();
            }
            Some(old) => {
                for k in 0..3 {
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

    /// `MaximizerProcessor::process`: with a side-chain, the side-chain filter runs over
    /// it first, and the detector takes it instead of the input.
    fn process_sc(&mut self, l: &mut [f32], r: &mut [f32], sc: Option<(&[f32], &[f32])>) {
        let n = l.len().min(r.len());
        let [sl, sr] = &mut self.sc;
        let sc = sc.map(|(a, b)| {
            sl.clear();
            sl.extend_from_slice(&a[..n]);
            sr.clear();
            sr.extend_from_slice(&b[..n]);
            self.scf.process(sl, sr);
            (&sl[..], &sr[..])
        });
        for (i, (xl, xr)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
            (*xl, *xr) = self.core.process((*xl, *xr), sc.map(|(a, b)| (a[i], b[i])));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(amount: f64, curve: f64, turbo: bool) -> Vec<(String, f64)> {
        vec![("amount".into(), amount), ("curve".into(), curve), ("turbo".into(), turbo as u8 as f64)]
    }

    fn run(fx: &mut Maximizer, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (l.to_vec(), r.to_vec());
        fx.process(&mut l, &mut r);
        (l, r)
    }

    #[test]
    fn amount_zero_is_a_delayed_tanh() {
        let mut fx = Maximizer::new(48000.0);
        fx.set(&params(0.0, 1.0, false));
        let x: Vec<f32> = (0..2000).map(|i| 0.1 * (i as f32 * 0.05).sin()).collect();
        let (l, _) = run(&mut fx, &x, &x);
        // Quiet input: env < a = 1, so the gain is 1 and the output is tanh(x), 1 late.
        for i in 1..x.len() {
            assert!((l[i] - x[i - 1].tanh()).abs() < 2e-4, "{i}: {} {}", l[i], x[i - 1].tanh());
        }
    }

    #[test]
    fn amount_one_raises_quiet_signals_and_stays_bounded() {
        let mut fx = Maximizer::new(44100.0);
        fx.set(&params(1.0, 1.0, false));
        let x: Vec<f32> = (0..44100).map(|i| 0.01 * (i as f32 * 0.03).sin()).collect();
        let (l, r) = run(&mut fx, &x, &x);
        let pk = l[20000..].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(pk > 0.5 && pk <= 1.0, "{pk}");
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() <= 1.0));
    }

    #[test]
    fn turbo_keeps_one_sample_of_delay() {
        let mut fx = Maximizer::new(48000.0);
        fx.set(&params(0.0, 1.0, true));
        let mut x = vec![0f32; 16];
        x[0] = 0.05;
        let (l, _) = run(&mut fx, &x, &x);
        assert_eq!(l[0], 0.0);
        assert!((l[1] - 0.05f32.tanh()).abs() < 1e-4, "{}", l[1]);
    }

    #[test]
    fn stereo_channels_stay_separate_with_a_linked_gain() {
        let mut fx = Maximizer::new(48000.0);
        fx.set(&params(0.5, 0.5, false));
        let x: Vec<f32> = (0..4800).map(|i| 0.3 * (i as f32 * 0.02).sin()).collect();
        let z = vec![0f32; x.len()];
        let (l, r) = run(&mut fx, &x, &z);
        assert!(r.iter().all(|v| *v == 0.0));
        assert!(l.iter().any(|v| *v != 0.0));
    }

    #[test]
    fn silence_and_parameter_changes_stay_finite() {
        let mut fx = Maximizer::new(96000.0);
        fx.set(&params(1.0, 0.0, true));
        let z = vec![0f32; 96000];
        let (l, _) = run(&mut fx, &z, &z);
        assert!(l.iter().all(|v| *v == 0.0));
        fx.set(&params(0.2, 1.0, false));
        let x: Vec<f32> = (0..9600).map(|i| 4.0 * (i as f32 * 0.1).sin()).collect();
        let (l, _) = run(&mut fx, &x, &x);
        assert!(l.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    }
}
