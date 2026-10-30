//! Maschine's Ice (docs/fx-iceverb.md): `RT::IceverbProcessor` → `FXCORE::IceverbCore`, a
//! compiled Reaktor-Core kernel. The mono sum of the input goes through two cascaded
//! band-pass biquads tuned by Color (resonance from Ice), then a chain of 0.5-gain
//! Schroeder all-passes and a feedback delay whose lengths scale with Size. The left wet
//! signal is taken inside the loop, the right one at the loop delay's output. The eight
//! derived parameters glide linearly over about 100 ms. All arithmetic follows the
//! assembly operation by operation (`f32`, `mul_add` where the kernel uses `fmadd`), so
//! the output can match bit for bit.

use super::{Effect, exp_e, param};

/// Delay-line length and index mask (+0x44 / +0x48).
const LEN: usize = 16384;
const MASK: u32 = 16383;
/// Longest delay read, in samples (+0x4c, `0x467ff800`).
const MAX_DELAY: f32 = 16382.0;
/// Control period of the parameter ramps, in samples (+0x2c).
const PERIOD: i32 = 128;
/// The kernel's denormal guard (`0x179abe15`, 1e-24).
const DC: f32 = f32::from_bits(0x179a_be15);
/// Ramps whose squared per-sample step is below this jump straight to the target.
const SNAP: f64 = f64::from_bits(0x3cd2_03af_9ee7_5616); // 1e-15
/// Lengths of the six all-passes, in units of the Size delay (§3.5).
const AP_LEN: [f32; 6] = [27.0, 8.0, 78.0, 27.0, 13.0, 29.0];

/// `IceverbCore::s_fGainTable` (0x10dc23b08): the band-pass input gain by Color pitch
/// (index = MIDI note), copied as stored.
const GAIN: [u32; 128] = [
    0x3eb126e9, 0x3eb33850, 0x3eb549d8, 0x3eb75b3e, 0x3eb96cc6, 0x3ebb7e2c, 0x3ebd8fb4, 0x3ebfa11b, 0x3ec1b2a2, 0x3ec3c409, 0x3ec5d591,
    0x3ec7e6f7, 0x3ec9f87f, 0x3ecc09e5, 0x3ece1b6d, 0x3ed02cd4, 0x3ed23e5c, 0x3ed44fc2, 0x3ed6614a, 0x3ed872b0, 0x3eda8438, 0x3edc959e,
    0x3edea726, 0x3ee0b88d, 0x3ee2ca15, 0x3ee4db7b, 0x3ee6236c, 0x3ee797ab, 0x3ee938de, 0x3eeb07af, 0x3eed04a3, 0x3eef3040, 0x3ef18b0d,
    0x3ef4156e, 0x3ef6cfc8, 0x3ef9bac3, 0x3efcd65e, 0x3f0011b2, 0x3f01d0ea, 0x3f03a92a, 0x3f059a74, 0x3f07a51a, 0x3f09c91d, 0x3f0c06af,
    0x3f0e5e03, 0x3f10cf29, 0x3f135a53, 0x3f15ff93, 0x3f18bf0a, 0x3f1b98e9, 0x3f1e8d33, 0x3f219c17, 0x3f24c5b9, 0x3f280a18, 0x3f2b6966,
    0x3f2ee3b4, 0x3f327925, 0x3f3629c7, 0x3f39f5be, 0x3f3ddd2b, 0x3f41dffc, 0x3f45fe76, 0x3f4a3897, 0x3f4e8e82, 0x3f530047, 0x3f578de7,
    0x3f5c37a4, 0x3f60fd6d, 0x3f65df65, 0x3f6addad, 0x3f6ff834, 0x3f752f2b, 0x3f7a82a5, 0x3f7ff2b2, 0x3f7fef28, 0x3f7feb9e, 0x3f7fe804,
    0x3f7fe458, 0x3f7fe08b, 0x3f7fdcbe, 0x3f7fd8cf, 0x3f7fd4d0, 0x3f7fd0c0, 0x3f7fcc8e, 0x3f7fc84b, 0x3f7fc3f8, 0x3f7ffaa0, 0x3f7ffb9c,
    0x3f7ffc33, 0x3f7ffc98, 0x3f7ffceb, 0x3f7ffd1e, 0x3f7ffd61, 0x3f7ffd93, 0x3f7ffdb5, 0x3f7ffde7, 0x3f7ffe09, 0x3f7ffe2a, 0x3f7ffe4c,
    0x3f7ffe4c, 0x3f7ffe5d, 0x3f7ffe6d, 0x3f7ffe7e, 0x3f7ffe8f, 0x3f7ffea0, 0x3f7ffeb0, 0x3f7c9d5a, 0x3f797311, 0x3f767ed8, 0x3f73bf94,
    0x3f713437, 0x3f6edba5, 0x3f6cb4c0, 0x3f68f5c3, 0x3f666666, 0x3f63d70a, 0x3f6147ae, 0x3f5eb852, 0x3f5c28f6, 0x3f59999a, 0x3f570a3d,
    0x3f547ae1, 0x3f547ae1, 0x3f547ae1, 0x3f547ae1, 0x3f547ae1, 0x3f547ae1, 0x00000000,
];

fn gain(i: usize) -> f32 {
    f32::from_bits(GAIN[i])
}

/// `round()` as the kernel does it: add ±0.5 (by the sign of the `f32`) in `f64`, truncate.
fn round_away(x: f32) -> i32 {
    let h = if x < 0.0 { -0.5 } else { 0.5 };
    (h + x as f64) as i32
}

/// Indices of the eight ramped values (+0xf0 current, +0x110 step, +0x130 target).
const DRY: usize = 0;
const WET: usize = 1;
const IN_GAIN: usize = 2;
const OMEGA: usize = 3;
const DAMP: usize = 4;
const FEEDBACK: usize = 5;
const LOOP: usize = 6;
const UNIT: usize = 7;

/// `FXCORE::IceverbCore`'s state (§3).
struct Core {
    sr: f32,
    /// Event inputs (normalized): Color, Ice, Size, Mix.
    color: f32,
    ice: f32,
    size: f32,
    mix: f32,
    /// Coefficients must be computed on the next sample (+0x28).
    init: bool,
    ctr: i32,
    /// Color as a MIDI pitch, 30..120 (+0x34).
    pitch: f32,
    f_lo: f32,
    f_hi: f32,
    /// 2π/sr (+0x40).
    w_k: f32,
    widx: u32,
    /// Band-pass coefficients (+0x58) for x[n], x[n−2], y[n−1], y[n−2].
    c: [f32; 4],
    /// Biquad history (+0x68..+0x84): `_1` one sample back, `_2` two; `u` is the second
    /// stage's input (= y1).
    x2: f32,
    y1_2: f32,
    u2: f32,
    y2_2: f32,
    x1: f32,
    y1_1: f32,
    u1: f32,
    y2_1: f32,
    /// Six all-pass lines and the loop delay (+0x88..+0xb8).
    bufs: Box<[[f32; LEN]; 7]>,
    /// The last value read from each line (+0xc0..+0xd8).
    z: [f32; 7],
    cur: [f32; 8],
    inc: [f32; 8],
    tgt: [f32; 8],
    /// A parameter event arrived (+0x150); a ramp is running (+0x151).
    changed: bool,
    ramping: bool,
    ramp_ctr: i32,
    ramp_steps: i32,
    ramp_k: f32,
}

impl Core {
    fn new(sr: f32) -> Core {
        Core {
            sr,
            color: 0.0,
            ice: 0.0,
            size: 0.0,
            mix: 0.0,
            init: true,
            ctr: PERIOD,
            pitch: 0.0,
            f_lo: 0.0,
            f_hi: 0.0,
            w_k: 0.0,
            widx: 0,
            c: [0.0; 4],
            x2: 0.0,
            y1_2: 0.0,
            u2: 0.0,
            y2_2: 0.0,
            x1: 0.0,
            y1_1: 0.0,
            u1: 0.0,
            y2_1: 0.0,
            bufs: vec![[0.0; LEN]; 7].into_boxed_slice().try_into().unwrap(),
            z: [0.0; 7],
            cur: [0.0; 8],
            inc: [0.0; 8],
            tgt: [0.0; 8],
            changed: false,
            ramping: false,
            ramp_ctr: 0,
            ramp_steps: 1,
            ramp_k: 0.0,
        }
    }

    /// `IceverbProcessor::resume` = `beginInit(sr)` + `endInit()` (§2, §3.2): clear the
    /// state, derive every target from the inputs and start there (no ramp).
    fn resume(&mut self) {
        let sr = self.sr;
        for b in self.bufs.iter_mut() {
            b.fill(0.0);
        }
        self.z = [0.0; 7];
        (self.x2, self.y1_2, self.u2, self.y2_2) = (0.0, 0.0, 0.0, 0.0);
        (self.x1, self.y1_1, self.u1, self.y2_1) = (0.0, 0.0, 0.0, 0.0);
        self.inc = [0.0; 8];
        self.widx = 0;
        self.init = true;
        self.ctr = PERIOD;
        self.ramping = false;
        self.ramp_ctr = 0;

        let steps = ((sr * 0.1) / PERIOD as f32 + 0.5) as i32;
        self.ramp_steps = if steps > 1 { steps } else { 1 };
        self.ramp_k = 1.0 / (self.ramp_steps * PERIOD) as f32;
        self.f_lo = sr / 24576.0;
        self.f_hi = sr / 2.125;
        self.w_k = f32::from_bits(0x40c9_0fe4) / sr; // 6.2831898
        self.ev_size();
        self.ev_color();
        self.ev_ice();
        self.ev_mix();
        self.cur = self.tgt;
        self.changed = true;
    }

    /// `processEventInSize`: the loop length factor and the delay unit (samples).
    fn ev_size(&mut self) {
        let s = self.size;
        self.tgt[LOOP] = (s * (2.0 - s)).mul_add(98.0, 12.0);
        let k = s.mul_add(f32::from_bits(0x3f66_6666), f32::from_bits(0x3dcc_cccd)); // 0.9, 0.1
        self.tgt[UNIT] = (k * self.sr) * f32::from_bits(0x3a83_126f); // 0.001
        self.changed = true;
    }

    /// `processEventInColor`: band-pass frequency, and the input gain (it reads the pitch).
    fn ev_color(&mut self) {
        self.pitch = self.color.mul_add(90.0, 30.0);
        let mut f = exp_e(self.pitch.mul_add(5.0, f32::from_bits(0x44ac_bc36))); // 1381.8816
        if f < self.f_lo {
            f = self.f_lo;
        }
        if f > self.f_hi {
            f = self.f_hi;
        }
        self.tgt[OMEGA] = self.w_k * f;
        self.in_gain();
        self.changed = true;
    }

    /// `processEventInIce`: band-pass damping, loop feedback and the input gain.
    fn ev_ice(&mut self) {
        let i = self.ice;
        self.tgt[DAMP] = 1.0 - i.mul_add(f32::from_bits(0x3f4c_cccd), f32::from_bits(0x3dcc_cccd));
        self.tgt[FEEDBACK] = (i * (2.0 - i)).mul_add(f32::from_bits(0x3f28_f5c3), f32::from_bits(0x3e4c_cccd));
        self.in_gain();
        self.changed = true;
    }

    /// The band-pass input gain (+0x138), shared by the Color and Ice events: a boost for
    /// Ice² above 0.8, times the pitch's table gain.
    fn in_gain(&mut self) {
        let i2 = self.ice * self.ice;
        let boost = if i2 <= f32::from_bits(0x3f4c_cccd) {
            f32::from_bits(0x3f8f_5c29) // 1.12
        } else {
            let t = (i2 + f32::from_bits(0xbf4c_cccd)) * f32::from_bits(0x40a0_0001);
            (((t * t) * f32::from_bits(0x3f28_f5c3)) as f64 + f64::from_bits(0x3ff1_eb85_1eb8_51ec)) as f32
        };
        let p = self.pitch;
        let i = round_away(p + -0.5);
        let fr = p - i as f32;
        let (a, b) = (gain(i as usize), gain(i as usize + 1));
        self.tgt[IN_GAIN] = boost * (b - a).mul_add(fr, a);
    }

    /// `processEventInMix`: dry falls over the upper half, wet (+3 dB) rises over the lower.
    fn ev_mix(&mut self) {
        let m = self.mix;
        let dm = 1.0 - m;
        self.tgt[DRY] = if dm > 0.5 { 1.0 } else { dm + dm };
        let w = f32::from_bits(0x3fb4_ce07); // 1.4125375
        self.tgt[WET] = if m > 0.5 { w } else { (m + m) * w };
        self.changed = true;
    }

    /// The control block of `processAudio` (§3.3), every 128 samples.
    fn control(&mut self) {
        if self.changed {
            self.changed = false;
            let mut any = false;
            for ((c, d), t) in self.cur.iter_mut().zip(self.inc.iter_mut()).zip(&self.tgt) {
                let s = (t - *c) * self.ramp_k;
                if ((s * s) as f64) < SNAP {
                    *d = 0.0;
                    *c = *t;
                } else {
                    *d = s;
                    any = true;
                }
            }
            if any {
                self.ramp_ctr = self.ramp_steps;
                self.ramping = true;
            }
        } else {
            self.ramp_ctr = self.ramp_ctr.wrapping_sub(1);
            if self.ramp_ctr == 0 {
                self.cur = self.tgt;
                self.inc = [0.0; 8];
                self.ramping = false;
            }
        }
    }

    /// Band-pass coefficients from the current frequency and damping (§3.4): sin/cos by
    /// polynomials, `alpha = damp·sin w`, normalized by `1 + alpha`.
    fn coefs(&mut self) {
        let f = f32::from_bits;
        let w = self.cur[OMEGA];
        let w2 = w * w;
        let mut s = w2.mul_add(f(0xb2cd_4ca8), f(0x3638_b797));
        s = w2.mul_add(s, f(0xb950_0c03));
        s = w2.mul_add(s, f(0x3c08_8885));
        s = w2.mul_add(s, f(0xbe2a_aac1));
        s = w2.mul_add(s, 1.0);
        let sin = w * s;
        let mut c = w2.mul_add(f(0xb48b_dad9), f(0x37cf_b5a1));
        c = w2.mul_add(c, f(0xbab6_09bd));
        c = w2.mul_add(c, f(0x3d2a_aa99));
        c = w2.mul_add(c, f(0xbeff_f5e8));
        let cos = w2.mul_add(c, 1.0);
        let alpha = self.cur[DAMP] * sin;
        let a = 1.0 / (alpha + 1.0);
        self.c = [alpha * a, -alpha * a, (cos + cos) * a, -(1.0 - alpha) * a];
    }

    /// A read of line `line`, `d` samples (clamped, rounded) behind write position `w`.
    fn read(&self, line: usize, w: u32, d: f32) -> f32 {
        let d = if d < 0.0 { 0.0 } else { d };
        let d = if d > MAX_DELAY { MAX_DELAY } else { d };
        let n = round_away(d);
        self.bufs[line][(w.wrapping_sub(n as u32) & MASK) as usize]
    }

    /// One 0.5-gain all-pass on line `k`: returns its output and reads the next `z[k]`.
    fn allpass(&mut self, k: usize, w: u32, x: f32) -> f32 {
        let y = x.mul_add(0.5, self.z[k]) + DC;
        self.bufs[k][w as usize] = y.mul_add(-0.5, x);
        self.z[k] = self.read(k, w, AP_LEN[k] * self.cur[UNIT]);
        y
    }

    /// `IceverbCore::processAudio` for one sample (§3.3–3.6).
    fn tick(&mut self, xl: f32, xr: f32) -> (f32, f32) {
        self.ctr += 1;
        if self.ctr >= PERIOD {
            self.control();
            self.ctr = 0;
        }
        for (c, d) in self.cur.iter_mut().zip(&self.inc) {
            *c += d;
        }
        if self.init || (self.ramping && (self.inc[OMEGA] != 0.0 || self.inc[DAMP] != 0.0)) {
            self.coefs();
            self.init = false;
        }

        // Two cascaded band-passes on the mono sum.
        let x = (xl + xr) * self.cur[IN_GAIN];
        let [c0, c1, c2, c3] = self.c;
        let y1 = c3.mul_add(self.y1_2, c2.mul_add(self.y1_1, c0.mul_add(x, c1 * self.x2))) + DC;
        let y2 = c3.mul_add(self.y2_2, c2.mul_add(self.y2_1, c0.mul_add(y1, c1 * self.u2))) + DC;
        (self.x2, self.y1_2, self.u2, self.y2_2) = (self.x1, self.y1_1, self.u1, self.y2_1);
        (self.x1, self.y1_1, self.u1, self.y2_1) = (x, y1, y1, y2);

        // Two diffusers, then the loop: four all-passes and a delay, fed back.
        let w = self.widx.wrapping_add(1) & MASK;
        self.widx = w;
        let o1 = self.allpass(0, w, y2);
        let o2 = self.allpass(1, w, o1);
        let o3 = self.allpass(2, w, self.z[6].mul_add(self.cur[FEEDBACK], o2));
        let o4 = self.allpass(3, w, o3);
        let o5 = self.allpass(4, w, o4);
        let o6 = self.allpass(5, w, o5);
        self.bufs[6][w as usize] = o6;
        self.z[6] = self.read(6, w, self.cur[LOOP] * self.cur[UNIT]);

        let (dry, wet) = (self.cur[DRY], self.cur[WET]);
        let r = (self.z[6] * wet) * f32::from_bits(0x3f99_999a); // 1.2
        (xl.mul_add(dry, o5 * wet), xr.mul_add(dry, r))
    }
}

/// The Ice effect: the parameters (`color`, `ice`, `size`, `mix`) go to the core as events.
pub struct Iceverb {
    core: Core,
    started: bool,
}

impl Iceverb {
    pub fn new(sr: f64) -> Self {
        Iceverb { core: Core::new(sr as f32), started: false }
    }
}

impl Effect for Iceverb {
    fn set(&mut self, params: &[(String, f64)]) {
        let c = &mut self.core;
        let color = param(params, "color", 0.75) as f32;
        let ice = param(params, "ice", 0.75) as f32;
        let size = param(params, "size", 0.5) as f32;
        let mix = param(params, "mix", 0.5) as f32;
        if !self.started {
            // The processor resumes with the stored values (§2).
            self.started = true;
            (c.color, c.ice, c.size, c.mix) = (color, ice, size, mix);
            c.resume();
            return;
        }
        // Maschine sends an event only for a value that changed; a repeated event would
        // restart a running ramp from where it is.
        if color != c.color {
            c.color = color;
            c.ev_color();
        }
        if ice != c.ice {
            c.ice = ice;
            c.ev_ice();
        }
        if size != c.size {
            c.size = size;
            c.ev_size();
        }
        if mix != c.mix {
            c.mix = mix;
            c.ev_mix();
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

    fn p(color: f64, ice: f64, size: f64, mix: f64) -> Vec<(String, f64)> {
        vec![("color".into(), color), ("ice".into(), ice), ("size".into(), size), ("mix".into(), mix)]
    }

    fn noise(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i * 7919 + 13) % 1000) as f32 / 1000.0 - 0.5).collect()
    }

    fn run(fx: &mut Iceverb, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut a, mut b) = (l.to_vec(), r.to_vec());
        for (x, y) in a.chunks_mut(512).zip(b.chunks_mut(512)) {
            fx.process(x, y);
        }
        (a, b)
    }

    #[test]
    fn targets_follow_the_kernel_curves() {
        let mut c = Core::new(44_100.0);
        (c.color, c.ice, c.size, c.mix) = (0.5, 0.0, 1.0, 1.0);
        c.resume();
        assert_eq!(c.pitch, 75.0);
        // MIDI 75 ≈ 622 Hz.
        let hz = c.tgt[OMEGA] / c.w_k;
        assert!((hz - 622.25).abs() < 0.5, "{hz}");
        assert_eq!(c.tgt[LOOP], 110.0);
        assert!((c.tgt[UNIT] - 44.1).abs() < 1e-4);
        assert_eq!((c.tgt[DRY], c.tgt[WET]), (0.0, f32::from_bits(0x3fb4_ce07)));
        assert_eq!(c.ramp_steps, 34);
    }

    #[test]
    fn defaults_reverberate_and_stay_finite() {
        let mut fx = Iceverb::new(44_100.0);
        fx.set(&[]);
        let mut x = vec![0.0; 88_200];
        x[0] = 1.0;
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        // A tail long after the impulse, on both sides.
        let tail = |v: &[f32]| v[22_050..].iter().map(|s| s * s).sum::<f32>();
        assert!(tail(&l) > 1e-6 && tail(&r) > 1e-6);
    }

    #[test]
    fn dry_mix_is_the_identity() {
        let mut fx = Iceverb::new(48_000.0);
        fx.set(&p(0.3, 1.0, 1.0, 0.0));
        let x = noise(20_000);
        let (l, r) = run(&mut fx, &x, &x);
        assert_eq!(l, x);
        assert_eq!(r, x);
    }

    #[test]
    fn silence_stays_silent_and_does_not_go_denormal() {
        let mut fx = Iceverb::new(44_100.0);
        fx.set(&p(1.0, 0.3, 0.2, 1.0));
        let x = noise(10_000);
        run(&mut fx, &x, &x);
        let z = vec![0.0; 400_000];
        let (l, r) = run(&mut fx, &z, &z);
        for v in l[300_000..].iter().chain(&r[300_000..]) {
            assert!(*v == 0.0 || v.abs() >= f32::MIN_POSITIVE, "{v}");
            assert!(v.abs() < 1e-6);
        }
    }

    #[test]
    fn set_keeps_state_and_ramps() {
        let x = noise(40_000);
        let mut a = Iceverb::new(44_100.0);
        a.set(&p(0.4, 0.6, 0.5, 0.5));
        let (l1, _) = run(&mut a, &x[..10_000], &x[..10_000]);
        a.set(&p(0.4, 0.6, 0.5, 0.5));
        let (l2, _) = run(&mut a, &x[10_000..], &x[10_000..]);
        let mut b = Iceverb::new(44_100.0);
        b.set(&p(0.4, 0.6, 0.5, 0.5));
        let (lb, _) = run(&mut b, &x, &x);
        assert_eq!([l1, l2].concat(), lb);

        // Mix 0 → 1 glides over 34 · 128 samples.
        let mut c = Iceverb::new(44_100.0);
        c.set(&p(0.4, 0.6, 0.5, 0.0));
        run(&mut c, &x[..1000], &x[..1000]);
        c.set(&p(0.4, 0.6, 0.5, 1.0));
        run(&mut c, &x[..1000], &x[..1000]);
        assert!(c.core.cur[DRY] > 0.0 && c.core.cur[DRY] < 1.0, "{}", c.core.cur[DRY]);
        run(&mut c, &x[..5000], &x[..5000]);
        assert_eq!(c.core.cur[DRY], 0.0);
        assert!(!c.core.ramping);
    }

    #[test]
    fn extreme_settings_stay_bounded() {
        for sr in [44_100.0, 96_000.0, 192_000.0] {
            let mut fx = Iceverb::new(sr);
            fx.set(&p(1.0, 1.0, 1.0, 1.0));
            let x: Vec<f32> = noise(200_000).iter().map(|v| v * 4.0).collect();
            let (l, r) = run(&mut fx, &x, &x);
            assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 100.0));
        }
    }
}
