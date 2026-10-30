//! Maschine's Transient Master (docs/fx-transient-master.md): `DATA::TransientDesigner` →
//! `RT::KontaktDSPCoreWrapper<NI::DSP::TranDes>`, Kontakt's transient designer. A
//! stereo-linked peak detector (after a 10 Hz high-pass) is compressed by `x^0.1` and drives
//! six one-pole envelope followers. A fast-minus-slow pair, high-passed at 4 Hz, gives the
//! transient envelope that Attack scales; the slow followers give the sustain envelope that
//! Sustain scales. The sum (in dB, plus Input Gain) sets one gain for both channels; the
//! result is high-passed at 10 Hz again, and with Limit on clipped to ±0.9999. Parameters
//! are applied at a 128-sample control rate. All arithmetic follows the assembly operation
//! by operation (`f32`, `mul_add` where the kernel uses `fmadd`/`fmsub`), so the output can
//! match bit for bit.

use super::{Effect, exp_e, param};

/// An `f32` constant as Maschine stores it.
const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// The wrapper's control block: `processControlRate` runs every 128 samples (§2).
const BLOCK: usize = 128;
/// `ln 2` (`0x3f317215`): the followers' coefficients are `ln2 / (sr·τ)`.
const LN2: f32 = c(0x3f31_7215);
/// Below this squared step (`f64`, `0x3cd203af9ee75616`) a parameter snaps (§3.2).
const SNAP: f64 = 1e-14;
/// The detector's peak is `x^0.1`-mapped by a table over 0.0005..3 (§4.1).
const POW_LO: f32 = c(0x3a03_126f); // 0.0005
const POW_HI: f32 = 3.0;
/// The output clip with Limit on (`0x3f7ff972`).
const CLIP: f32 = c(0x3f7f_f972); // 0.9999
/// dB → exp-table units: `x·log2(10)·60/20 + 1200` (`0x411f73da`).
const K_DB: f32 = c(0x411f_73da);
/// The followers' upper clamp.
const CEIL: f32 = 10000.0;

/// `NI::DSP::TranDes::s_cfmapVectorPower0p1`: `(i/10000)^0.1` in `f64`, rounded to `f32`
/// (equal to all 30002 stored entries).
fn pow_table() -> &'static [f32; 30002] {
    static T: std::sync::OnceLock<Box<[f32; 30002]>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = Box::new([0f32; 30002]);
        for (i, e) in t.iter_mut().enumerate() {
            *e = (i as f64 / 10000.0).powf(0.1) as f32;
        }
        t
    })
}

/// `x^0.1` as the kernel takes it: the table with linear interpolation on 0.0005..3,
/// `powf` elsewhere (and for NaN).
fn pow01(x: f32) -> f32 {
    if (POW_LO..=POW_HI).contains(&x) {
        let t = pow_table();
        let s = x * 10000.0;
        let i = s as i32;
        let fr = s - i as f32;
        let (a, b) = (t[i as usize], t[i as usize + 1]);
        fr.mul_add(b - a, a)
    } else {
        x.powf(c(0x3dcc_cccd))
    }
}

/// A first-order bilinear high-pass, `g = tan(arg)`: `(b0, b1, a1)` with `b1 = −b0`.
fn highpass(arg: f32) -> (f32, f32, f32) {
    let g = arg.tan();
    let a = 1.0 / (g + 1.0);
    (a, -a, (g + -1.0) * a)
}

/// One step of a first-order section: `b0·x + b1·x1 − a1·y1` (`fmadd`, then `fmsub`).
#[inline]
fn section((b0, b1, a1): (f32, f32, f32), x: f32, s: &mut (f32, f32)) -> f32 {
    let y = b0.mul_add(x, b1 * s.0);
    let y = (-a1).mul_add(s.1, y);
    *s = (x, y);
    y
}

/// A one-pole follower step: `env + k·(x − env)` (`fmadd`), `k` by the sign of `x − env`.
#[inline]
fn follow(env: &mut f32, x: f32, up: f32, down: f32) {
    let d = x - *env;
    let k = if d > 0.0 { up } else { down };
    *env = k.mul_add(d, *env);
}

/// `x` clamped to `0..=CEIL` as the kernel does it (`fcsel`: NaN passes).
#[inline]
fn clamp_ceil(x: f32) -> f32 {
    let x = if x < 0.0 { 0.0 } else { x };
    if x > CEIL { CEIL } else { x }
}

/// `NI::DSP::TranDes` (its fields with their object offsets; §3–§4).
struct TranDes {
    /// +0x08: suspended (parameters then apply at once).
    suspended: bool,
    /// +0x3c: Limit (the output clip).
    limit: bool,
    /// +0x44: output gain (1 after `onResume`).
    out_gain: f32,
    /// +0x48..+0x50: the 10 Hz high-pass, on the detector and on the output.
    hp10: (f32, f32, f32),
    /// Per-channel `(x1, y1)`: detector (+0x54/+0x94), output (+0xd4/+0x114).
    det: [(f32, f32); 2],
    out: [(f32, f32); 2],
    /// +0x154..+0x15c: the 4 Hz high-pass on the transient envelope; state +0x160/+0x164.
    hp1: (f32, f32, f32),
    hp1_s: (f32, f32),
    /// Follower coefficients `ln2/(sr·τ)` (+0x17c..+0x1b4, the ones these modes use).
    k_fast_up: f32,
    k_slow_up: f32,
    k_rel: f32,
    k_tr_down: f32,
    k_hold_down: f32,
    k_mid_up: f32,
    k_long_up: f32,
    k_long_down: f32,
    /// The six followers (+0x1b8..+0x1cc).
    env: [f32; 6],
    /// Smoothed parameters: current (+0x1e0), per-sample step (+0x200), target (+0x220).
    /// Slots: 0 Attack, 1 Sustain, 2 Input Gain (dB), 7 an output fade nothing reads.
    cur: [f32; 8],
    inc: [f32; 8],
    target: [f32; 8],
    /// +0x240: a parameter changed; +0x241: a ramp is running (each sample adds `inc`).
    changed: bool,
    ramping: bool,
    /// +0x244: control blocks left in the ramp; +0x248: its length in blocks (≈ 20 ms);
    /// +0x24c: `1/(n·128)`.
    counter: i32,
    ramp_n: i32,
    ramp_inv: f32,
}

impl TranDes {
    /// The followers' reset values (`0x10c3515d0`, `0x10c351720`).
    const ENV0: [f32; 6] = [c(0x3e86_24dd), c(0x3ed9_999a), 0.0, c(0x3ece_5604), c(0x3ece_5604), c(0x3f21_47ae)];

    fn new() -> TranDes {
        TranDes {
            suspended: true,
            limit: true,
            out_gain: 0.0,
            hp10: (0.0, 0.0, 0.0),
            det: [(0.0, 0.0); 2],
            out: [(0.0, 0.0); 2],
            hp1: (0.0, 0.0, 0.0),
            hp1_s: (0.0, 0.0),
            k_fast_up: 0.0,
            k_slow_up: 0.0,
            k_rel: 0.0,
            k_tr_down: 0.0,
            k_hold_down: 0.0,
            k_mid_up: 0.0,
            k_long_up: 0.0,
            k_long_down: 0.0,
            env: Self::ENV0,
            cur: [0.0; 8],
            inc: [0.0; 8],
            target: [0.0; 8],
            changed: false,
            ramping: false,
            counter: 0,
            ramp_n: 0,
            ramp_inv: 0.0,
        }
    }

    /// `onSetSampleRate(sr, 128)` (§3.1).
    fn set_sample_rate(&mut self, sr: f32) {
        let n = ((sr * c(0x3ca3_d70a)) / BLOCK as f32 + 0.5) as i32; // 20 ms in blocks
        self.ramp_n = if n > 1 { n } else { 1 };
        self.ramp_inv = 1.0 / (self.ramp_n * BLOCK as i32) as f32;
        let k = |tau: u32| LN2 / (sr * c(tau));
        self.k_fast_up = k(0x3745_f89d); // +0x17c: 11.8 µs
        self.k_rel = k(0x3d87_2b02); // +0x184 = +0x194: 66 ms
        self.k_slow_up = k(0x3bdd_d6e0); // +0x18c: 6.77 ms
        self.k_tr_down = k(0x3c03_9475); // +0x19c: 8.031 ms
        self.k_hold_down = k(0x3d4c_cccd); // +0x1a4 = +0x1ac: 50 ms
        self.k_mid_up = k(0x3a03_126f); // +0x1a8: 0.5 ms
        self.k_long_up = k(0x38fb_a882); // +0x1b0: 0.12 ms
        self.k_long_down = k(0x3fd9_999a); // +0x1b4: 1.7 s
        let inv = 1.0 / sr;
        self.hp10 = highpass(inv * c(0x41fb_53d2)); // π·10/sr
        self.hp1 = highpass(inv * c(0x4149_0fdb)); // π·4/sr
    }

    /// `onReset`: filter states and followers.
    fn reset(&mut self) {
        self.det = [(0.0, 0.0); 2];
        self.out = [(0.0, 0.0); 2];
        self.hp1_s = (0.0, 0.0);
        self.env = Self::ENV0;
    }

    /// `onResume`: the parameters jump to their targets, then the output fade's target
    /// becomes 1 (so the first control block starts a ramp).
    fn resume(&mut self) {
        self.suspended = false;
        self.cur = self.target;
        self.out_gain = 1.0;
        self.target[7] = 1.0;
        self.changed = true;
    }

    /// `setParameter(idx, v)`, `v` normalized 0..1 (§3.3).
    fn set_parameter(&mut self, idx: usize, v: f32) {
        match idx {
            0 => self.target[0] = ((v + -0.5) * 120.0) * 2.0,
            1 => self.target[1] = ((v + -0.5) * 115.0) * 2.0,
            2 => {
                let s = v + -0.5;
                self.target[2] = (s + s) * if v < 0.5 { 24.0 } else { 6.0 };
            }
            _ => self.limit = v > 0.5,
        }
        if idx <= 2 {
            self.changed = true;
        }
        if self.suspended {
            self.cur = self.target;
            self.changed = true;
        }
    }

    /// `processControlRate` (§3.2): start a ramp toward changed targets (`n` blocks
    /// long), or count one down and snap at its end.
    fn control(&mut self) {
        if self.changed {
            self.changed = false;
            let mut sq = [0f32; 8];
            for j in 0..8 {
                self.inc[j] = (self.target[j] - self.cur[j]) * self.ramp_inv;
            }
            for (j, q) in sq.iter_mut().enumerate() {
                let s = self.inc[j] * self.inc[j];
                if (s as f64) < SNAP {
                    self.inc[j] = 0.0;
                    self.cur[j] = self.target[j];
                } else {
                    *q = s;
                }
            }
            let sum = (((sq[0] + sq[1]) + sq[2]) + sq[3]) + (((sq[4] + sq[5]) + sq[6]) + sq[7]);
            if sum > 0.0 {
                self.counter = self.ramp_n;
                self.ramping = true;
            }
        } else {
            self.counter = self.counter.wrapping_sub(1);
            if self.counter == 0 {
                self.cur = self.target;
                self.inc = [0.0; 8];
                self.ramping = false;
            }
        }
    }

    /// `processAudio<Mode>` for the modes Maschine reaches: 0, 1 (ramping), 32 (Limit),
    /// 33 (§4).
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let hp10 = self.hp10;
        for i in 0..l.len() {
            if self.ramping {
                for j in 0..8 {
                    self.cur[j] += self.inc[j];
                }
            }
            // Peak of the 10 Hz high-passed input over both channels.
            let mut pk = 0f32;
            for (ch, x) in [l[i], r[i]].into_iter().enumerate() {
                let y = section(hp10, x, &mut self.det[ch]).abs();
                pk = if y < pk { pk } else { y };
            }
            let e = pow01(pk) * c(0x3f33_3333); // ·0.7
            let att = self.cur[0];
            let [e1, e2, e3, e4, e5, e6] = &mut self.env;
            // Transient: fast minus slow follower, shaped, clamped to 0..0.12, high-passed,
            // rectified, held by a follower with instant attack.
            follow(e1, e, self.k_fast_up, self.k_rel);
            follow(e2, e, self.k_slow_up, self.k_rel);
            let t = *e1 - *e2;
            let t = if t > c(0x3d98_5f07) { t * c(0x3f92_233a) } else { t }; // > 0.0744: ·1.1417
            let t = t + c(0xbd4e_703b); // −0.0504
            let t = if t < 0.0 { 0.0 } else { t };
            let t = if t > c(0x3df5_c28f) { c(0x3df5_c28f) } else { t }; // 0.12
            let a = section(self.hp1, t, &mut self.hp1_s).abs();
            let d = a - *e3;
            *e3 = if d > 0.0 { a } else { self.k_tr_down.mul_add(d, *e3) };
            let att_term = att * *e3;
            // Sustain: a peak-hold minus a medium follower, and a long follower minus both.
            let d = e - *e4;
            *e4 = if d > 0.0 { e } else { self.k_hold_down.mul_add(d, *e4) };
            follow(e5, e, self.k_mid_up, self.k_hold_down);
            let u = clamp_ceil((*e4 - *e5) + c(0xbd23_d70a)); // −0.04
            follow(e6, e, self.k_long_up, self.k_long_down);
            let w = clamp_ceil((*e6 - *e5) - u);
            let sus = self.cur[1];
            let s = (sus * c(0xbea8_f5c3)).mul_add(*e3, sus * w); // −0.33
            let db = (att_term + self.cur[2]) + s;
            let g = exp_e(db.mul_add(K_DB, 1200.0));
            // The gain, the output 10 Hz high-pass, the clip.
            for (ch, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let y = section(hp10, g * *x, &mut self.out[ch]);
                let y = if self.limit {
                    let y = if y < -CLIP { -CLIP } else { y };
                    if y > CLIP { CLIP } else { y }
                } else {
                    y
                };
                *x = self.out_gain * y;
            }
        }
    }
}

/// Field names and defaults (stored values) of `DATA::TransientDesigner`, in `TranDes`
/// parameter order (0 Attack, 1 Sustain, 2 Input Gain, 3 Limit).
const PARAMS: [(&str, f32); 4] = [("attack", 0.0), ("sustain", 0.0), ("input_gain", 0.0), ("limit", 1.0)];

pub struct TransientMaster {
    core: TranDes,
    /// The wrapper's samples left before the next control block (+0xa8).
    left: usize,
    values: Option<[f32; 4]>,
}

impl TransientMaster {
    pub fn new(sr: f64) -> Self {
        let mut core = TranDes::new();
        core.set_sample_rate(sr as f32);
        TransientMaster { core, left: 0, values: None }
    }

    /// `KontaktDSPCoreWrapper::processParameter` with the stored value normalized
    /// (`NumericParameter::getNormalized`: `(v − min)·(1/(max − min))`, in `f64`).
    fn apply(&mut self, k: usize, v: f32) {
        let n = if k == 3 { if v != 0.0 { 1.0 } else { 0.0 } } else { ((v - -1.0) as f64 * 0.5) as f32 };
        self.core.set_parameter(k, n);
    }
}

impl Effect for TransientMaster {
    fn set(&mut self, params: &[(String, f64)]) {
        let new = PARAMS.map(|(name, def)| param(params, name, def as f64) as f32);
        match self.values {
            None => {
                // The first settings reach the core before it resumes.
                for (k, v) in new.iter().enumerate() {
                    self.apply(k, *v);
                }
                self.core.reset();
                self.core.resume();
            }
            Some(old) => {
                for k in 0..4 {
                    if new[k].to_bits() != old[k].to_bits() {
                        self.apply(k, new[k]);
                    }
                }
            }
        }
        self.values = Some(new);
    }

    /// `KontaktDSPCoreWrapper::process`: control blocks of 128 samples, across calls.
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n = l.len().min(r.len());
        let mut i = 0;
        while i < n {
            if self.left == 0 {
                self.core.control();
                self.left = BLOCK;
            }
            let m = self.left.min(n - i);
            self.core.process(&mut l[i..i + m], &mut r[i..i + m]);
            self.left -= m;
            i += m;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(attack: f64, sustain: f64, gain: f64, limit: bool) -> Vec<(String, f64)> {
        [("attack", attack), ("sustain", sustain), ("input_gain", gain), ("limit", if limit { 1.0 } else { 0.0 })]
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect()
    }

    fn run(fx: &mut TransientMaster, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (l.to_vec(), r.to_vec());
        for (a, b) in l.chunks_mut(300).zip(r.chunks_mut(300)) {
            fx.process(a, b);
        }
        (l, r)
    }

    /// Decaying 200 Hz hits every 0.25 s.
    fn hits(n: usize, sr: f32) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = (i % (sr as usize / 4)) as f32 / sr;
                (t * 200.0 * std::f32::consts::TAU).sin() * 0.5 * (-t * 30.0).exp()
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn pow_table_matches_powf() {
        for i in 1..3000 {
            let x = i as f32 * 0.001;
            assert!((pow01(x) - x.powf(0.1)).abs() < 1e-5, "{x}");
        }
        assert_eq!(pow01(0.0), 0.0);
    }

    #[test]
    fn neutral_is_only_a_highpass() {
        // Attack, Sustain, Gain 0: unity gain; only the 10 Hz high-pass remains.
        let mut fx = TransientMaster::new(48_000.0);
        fx.set(&params(0.0, 0.0, 0.0, false));
        let x: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let (l, r) = run(&mut fx, &x, &x);
        assert!((rms(&l[24_000..]) / rms(&x[24_000..]) - 1.0).abs() < 1e-3);
        assert_eq!(l, r);
    }

    #[test]
    fn input_gain_is_in_db() {
        let mut fx = TransientMaster::new(44_100.0);
        fx.set(&params(0.0, 0.0, -0.25, false)); // −6 dB
        let x: Vec<f32> = (0..44_100).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let (l, _) = run(&mut fx, &x, &x);
        let db = 20.0 * (rms(&l[22_050..]) / rms(&x[22_050..])).log10();
        assert!((db + 6.0).abs() < 0.05, "{db}");
    }

    #[test]
    fn attack_and_sustain_shape_hits() {
        let sr = 48_000.0;
        let x = hits(48_000, sr);
        let level = |a: f64, s: f64, range: std::ops::Range<usize>| {
            let mut fx = TransientMaster::new(sr as f64);
            fx.set(&params(a, s, 0.0, false));
            let (l, _) = run(&mut fx, &x, &x);
            rms(&l[range])
        };
        // The first 2 ms of a hit (the transient), and its tail.
        let (head, tail) = (24_000..24_100, 26_000..30_000);
        let h0 = level(0.0, 0.0, head.clone());
        assert!(level(1.0, 0.0, head.clone()) > 1.5 * h0);
        assert!(level(-1.0, 0.0, head) < 0.7 * h0);
        let t0 = level(0.0, 0.0, tail.clone());
        assert!(level(0.0, 1.0, tail.clone()) > 1.2 * t0);
        assert!(level(0.0, -1.0, tail) < 0.8 * t0);
    }

    #[test]
    fn limit_clips() {
        let mut fx = TransientMaster::new(44_100.0);
        fx.set(&params(1.0, 1.0, 1.0, true));
        let x: Vec<f32> = (0..20_000).map(|i| (i as f32 * 0.05).sin() * 0.9).collect();
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.abs() <= CLIP));
        assert!(l.iter().any(|v| v.abs() == CLIP));
    }

    #[test]
    fn stays_finite_and_decays() {
        let mut fx = TransientMaster::new(44_100.0);
        fx.set(&params(1.0, -1.0, 1.0, false));
        let mut x = vec![0f32; 40_000];
        x[100] = 1.0;
        x[20_000..20_100].fill(8.0);
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        let z = vec![0f32; 400_000];
        let (l, _) = run(&mut fx, &z, &z);
        assert!(l[390_000..].iter().all(|v| v.abs() < 1e-6));
    }

    #[test]
    fn parameter_change_ramps_then_snaps() {
        let mut fx = TransientMaster::new(44_100.0);
        fx.set(&params(0.0, 0.0, 0.0, false));
        let x = vec![0.1f32; 4096];
        run(&mut fx, &x, &x);
        fx.set(&params(0.0, 0.0, -1.0, false));
        assert!(fx.core.target[2] == -24.0 && fx.core.cur[2] == 0.0);
        run(&mut fx, &x, &x);
        assert_eq!(fx.core.cur[2], -24.0);
        assert!(!fx.core.ramping);
    }
}
