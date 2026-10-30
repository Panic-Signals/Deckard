//! Maschine's Reflex (docs/fx-reflex.md): `RT::ReflexProcessor` → `FXCORE::ReflexCore`, a
//! compiled Reaktor-Core kernel. A small early-reflection reverb: the right input goes
//! through a gain and two band-passes ("Color"), then a chain of five lattice all-passes
//! whose delays scale with Size, with one feedback loop around the middle ("Smooth"); two
//! taps of it make the wet left and right. The left input reaches the wet signal only as a
//! faint filtered residue (§3.5). The four parameters glide linearly over ~20 ms, updated
//! every 128 samples. All `f32`; `mul_add` exactly where the kernel fuses (`fmadd`/`fmla`),
//! so the output can match bit for bit.

use super::{Effect, exp_e, param};

/// Delay-line length (the kernel masks indices with 0x3fff); seven lines.
const LEN: usize = 16384;
const MASK: i32 = 0x3fff;
/// Control period (samples). The counter starts at 129, so the first sample ticks.
const BLOCK: i32 = 128;
const CTR0: i32 = 129;
/// A glide step whose square is below this snaps to its target (`0x3cd203af9ee75616`).
const SNAP: f64 = 1e-15;

const C_LO: f32 = f32::from_bits(0x382a_aaab); // 4.0690105e-5 (1.79 Hz at 44.1 k)
const C_HI: f32 = f32::from_bits(0x3ef0_f0f1); // 0.47058824 (8/17)
const TWO_PI: f32 = f32::from_bits(0x40c9_0fe4); // 6.28319
const G1: f32 = f32::from_bits(0x3f41_e4f7); // 0.7574
const G2: f32 = f32::from_bits(0x3f33_afb8); // 0.7019
const G3: f32 = f32::from_bits(0x3f29_4af5); // 0.6613
const RESIDUE: f32 = f32::from_bits(0x3d23_0f14); // 0.0398093
const WET: f32 = f32::from_bits(0x3fb4_c73b); // 1.41233
const P09: f32 = f32::from_bits(0x3f66_6666); // 0.9
/// The seven delays, in Size units (sr·(0.1 … 1) ms each).
const TAPS: [f32; 7] = [21.0, 8.0, 23.0, 18.0, 43.0, 110.0, 39.0];

/// `ReflexCore::s_FbkArray`: Smooth → feedback gain, 128 entries (6-digit decimals).
/// The kernel also reads the word before it (0.0) at Smooth 0, and the one after it
/// (`0x01a1b370`, times a zero fraction) at Smooth 1.
const FBK: [f32; 128] = [
    7.94341e-15,
    1.40371e-11,
    1.50966e-09,
    3.71301e-08,
    3.81754e-07,
    2.2445e-06,
    9.03212e-06,
    2.77674e-05,
    7.00252e-05,
    0.000152017,
    0.000293808,
    0.000517979,
    0.000848233,
    0.00130742,
    0.00191736,
    0.00269656,
    0.00366173,
    0.00482456,
    0.00619509,
    0.00777702,
    0.00957612,
    0.0115913,
    0.0138208,
    0.0162588,
    0.0189031,
    0.0217411,
    0.0247773,
    0.0279866,
    0.031367,
    0.0349115,
    0.0386081,
    0.0424419,
    0.0464159,
    0.0505119,
    0.0547134,
    0.0590172,
    0.0634171,
    0.0679049,
    0.0724712,
    0.0771062,
    0.0818,
    0.0865427,
    0.0913505,
    0.096203,
    0.101083,
    0.105988,
    0.110916,
    0.115864,
    0.120828,
    0.125805,
    0.13079,
    0.13578,
    0.14077,
    0.145757,
    0.150736,
    0.155705,
    0.16066,
    0.165598,
    0.170516,
    0.175411,
    0.180312,
    0.185195,
    0.190048,
    0.19487,
    0.20355,
    0.215951,
    0.228136,
    0.240097,
    0.251827,
    0.263321,
    0.274572,
    0.285578,
    0.296337,
    0.306847,
    0.317111,
    0.327128,
    0.336903,
    0.346438,
    0.355757,
    0.364902,
    0.373816,
    0.382506,
    0.390981,
    0.399248,
    0.407315,
    0.415188,
    0.422873,
    0.430377,
    0.437704,
    0.444861,
    0.451853,
    0.458684,
    0.46536,
    0.471885,
    0.478264,
    0.484501,
    0.4906,
    0.496566,
    0.502402,
    0.508112,
    0.5137,
    0.519169,
    0.524523,
    0.529765,
    0.534898,
    0.539925,
    0.54485,
    0.549675,
    0.554403,
    0.559036,
    0.563578,
    0.56803,
    0.572396,
    0.576678,
    0.580877,
    0.584996,
    0.589038,
    0.593003,
    0.596896,
    0.600716,
    0.604466,
    0.608149,
    0.611764,
    0.615316,
    0.618804,
    0.622231,
    0.625598,
    0.628907,
];

fn fbk_entry(i: i32) -> f32 {
    match i {
        ..=-1 => 0.0,
        128.. => f32::from_bits(0x01a1_b370),
        _ => FBK[i as usize],
    }
}

/// `round()` as the kernel does it: add ±0.5 (by the sign of the `f32`) in `f64`, truncate.
fn round_away(x: f32) -> i32 {
    let h = if x < 0.0 { -0.5 } else { 0.5 };
    (h + x as f64) as i32
}

/// `__sincosf_stret`, the libm call the kernel makes. (Its cosine differs from `cosf` in
/// the last bit for about 0.7 % of inputs, so the port calls it too.)
#[cfg(target_vendor = "apple")]
fn sin_cos(x: f32) -> (f32, f32) {
    #[repr(C)]
    struct SinCos {
        s: f32,
        c: f32,
    }
    unsafe extern "C" {
        fn __sincosf_stret(x: f32) -> SinCos;
    }
    // SAFETY: a pure libSystem math function, no pointers involved.
    let r = unsafe { __sincosf_stret(x) };
    (r.s, r.c)
}

#[cfg(not(target_vendor = "apple"))]
fn sin_cos(x: f32) -> (f32, f32) {
    (x.sin(), x.cos())
}

/// `processEventInColr` (§1.5): (band-pass frequency in Hz, input gain).
fn color_targets(c: f32) -> (f32, f32) {
    let k = c * c.abs().mul_add(f32::from_bits(0xbe99_999a), f32::from_bits(0x3fa6_6666));
    let f = exp_e(k.mul_add(90.0, 30.0).mul_add(5.0, f32::from_bits(0x44ac_bc36)));
    let base = f32::from_bits(0x4020_c41e); // 2.51197
    let g =
        if k > f32::from_bits(0x3f28_f5c3) { ((k + f32::from_bits(0xbf28_f5c3)) * f32::from_bits(0x3fbc_3c4b) + 1.0) * base } else { base };
    (f, g)
}

/// `processEventInSm`: (band-pass damping 0.9·v, feedback gain from `FBK`).
fn smooth_targets(s: f32) -> (f32, f32) {
    let damp = s * P09;
    // Stored values are 0..1; the clamp only keeps other values inside the table.
    let i = round_away(s.mul_add(127.0, -0.5)).clamp(-1, 127);
    let fr = s.mul_add(127.0, -(i as f32));
    let (a, b) = (fbk_entry(i), fbk_entry(i + 1));
    (damp, (b - a).mul_add(fr, a))
}

/// `processEventInSize`: samples per delay unit, sr·(0.1 + 0.9·v)/1000.
fn size_target(v: f32, sr: f32) -> f32 {
    v.mul_add(P09, f32::from_bits(0x3dcc_cccd)) * sr * f32::from_bits(0x3a83_126f)
}

/// `processEventInMix`: (dry, wet) gains.
fn mix_targets(m: f32) -> (f32, f32) {
    let d = 1.0 - m;
    let dry = if d > 0.5 { 1.0 } else { d * 2.0 };
    let wet = if m > 0.5 { WET } else { (m + m) * WET };
    (dry, wet)
}

/// A two-lane direct-form-I biquad state.
#[derive(Clone, Copy, Default)]
struct Bq {
    x1: [f32; 2],
    x2: [f32; 2],
    y1: [f32; 2],
    y2: [f32; 2],
}

/// Band-pass coefficients (§3.3).
#[derive(Clone, Copy, Default)]
struct Coefs {
    a1: f32,
    a2: f32,
    b0: f32,
    b1: f32,
    b2: f32,
}

impl Coefs {
    /// From frequency `f` (Hz) and damping `d`: t = (1 − d)·sin w; a1 = 2cos w/(1+t),
    /// a2 = −(1−t)/(1+t), b0 = t/(1+t), b1 = 0, b2 = −t/(1+t).
    fn new(f: f32, d: f32, sr: f32) -> Coefs {
        let (lo, hi) = (sr * C_LO, sr * C_HI);
        let f = if f < lo { lo } else { f };
        let f = if f > hi { hi } else { f };
        let (s, c) = sin_cos(f * TWO_PI / sr);
        let t = (1.0 - d) * s;
        let n = 1.0 / (t + 1.0);
        Coefs { a1: (c + c) * n, a2: -((1.0 - t) * n), b0: t * n, b1: n * 0.0, b2: -(t * n) }
    }

    /// One step of both lanes, in the kernel's sum order.
    fn run(&self, s: &mut Bq, x: [f32; 2]) -> [f32; 2] {
        let mut y = [0f32; 2];
        for i in 0..2 {
            let ff = (x[i] * self.b0 + s.x1[i] * self.b1) + s.x2[i] * self.b2;
            y[i] = ff + (self.a1 * s.y1[i] + self.a2 * s.y2[i]);
        }
        s.x2 = s.x1;
        s.x1 = x;
        s.y2 = s.y1;
        s.y1 = y;
        y
    }
}

/// `FXCORE::ReflexCore`'s state (§3).
struct Core {
    sr: f32,
    /// Event inputs (normalized): Color, Smooth, Size, Mix.
    input: [f32; 4],
    ctr: i32,
    steps: i32,
    inv: f32,
    /// Glide lanes: 0 frequency, 1 gain, 2 damping, 3 feedback, 4 size, 5 dry, 6 wet,
    /// 7 unused. Current value, per-sample step, target.
    cur: [f32; 8],
    inc: [f32; 8],
    tgt: [f32; 8],
    changed: bool,
    ramping: bool,
    countdown: i32,
    co: Coefs,
    gain: f32,
    dry: f32,
    wet: f32,
    fbk: f32,
    size: f32,
    bq1: Bq,
    bq2: Bq,
    /// All-pass outputs v1..v5 and delayed reads r1..r6 (r6: the feedback tap).
    v: [f32; 5],
    r: [f32; 6],
    w: i32,
    /// Seven lines of `LEN`.
    bufs: Vec<f32>,
}

impl Core {
    fn new(sr: f32) -> Core {
        Core {
            sr,
            input: [0.0; 4],
            ctr: CTR0,
            steps: 0,
            inv: 0.0,
            cur: [0.0; 8],
            inc: [0.0; 8],
            tgt: [0.0; 8],
            changed: false,
            ramping: false,
            countdown: 0,
            co: Coefs::default(),
            gain: 0.0,
            dry: 0.0,
            wet: 0.0,
            fbk: 0.0,
            size: 0.0,
            bq1: Bq::default(),
            bq2: Bq::default(),
            v: [0.0; 5],
            r: [0.0; 6],
            w: 0,
            bufs: vec![0.0; 7 * LEN],
        }
    }

    /// `processEventIn{Colr,Sm,Size,Mix}` for input `p` (0..3).
    fn event(&mut self, p: usize) {
        let v = self.input[p];
        match p {
            0 => (self.tgt[0], self.tgt[1]) = color_targets(v),
            1 => (self.tgt[2], self.tgt[3]) = smooth_targets(v),
            2 => self.tgt[4] = size_target(v, self.sr),
            _ => (self.tgt[5], self.tgt[6]) = mix_targets(v),
        }
        self.changed = true;
    }

    /// `ReflexProcessor::resume` → `beginInit` + `endInit` (§2, §3.2): clear everything and
    /// take the parameters as they are (no glide).
    fn resume(&mut self) {
        let mut bufs = std::mem::take(&mut self.bufs);
        bufs.fill(0.0);
        *self = Core { input: self.input, bufs, ..Core::new(self.sr) };
        let steps = ((self.sr * 0.02) / BLOCK as f32 + 0.5) as i32;
        self.steps = if steps > 1 { steps } else { 1 };
        self.inv = 1.0 / (self.steps * BLOCK) as f32;
        for p in [3, 0, 1, 2] {
            self.event(p);
        }
        self.cur = self.tgt;
        self.gain = self.cur[1];
        self.dry = self.cur[5];
        self.wet = self.cur[6];
        self.fbk = self.cur[3];
        self.size = self.cur[4];
        self.co = Coefs::new(self.cur[0], self.cur[2], self.sr);
    }

    /// The glide update at the top of `processAudio` (§3.3).
    fn control(&mut self) {
        self.ctr += 1;
        if self.ctr < BLOCK {
            return;
        }
        self.ctr = 0;
        if self.changed {
            self.changed = false;
            let mut moving = false;
            for i in 0..8 {
                self.inc[i] = (self.tgt[i] - self.cur[i]) * self.inv;
                if ((self.inc[i] * self.inc[i]) as f64) < SNAP {
                    self.inc[i] = 0.0;
                    self.cur[i] = self.tgt[i];
                } else {
                    moving = true;
                }
            }
            if moving {
                self.countdown = self.steps;
                self.ramping = true;
            }
        } else {
            self.countdown = self.countdown.wrapping_sub(1);
            if self.countdown == 0 {
                // The glide ends on its targets, but the coefficients and gains keep the
                // values of the last gliding sample (they update only while gliding).
                self.cur = self.tgt;
                self.inc = [0.0; 8];
                self.ramping = false;
            }
        }
    }

    /// `ReflexCore::processAudio` for one sample (§3.3–3.5).
    fn tick(&mut self, xl: f32, xr: f32) -> (f32, f32) {
        self.control();
        for (c, d) in self.cur.iter_mut().zip(self.inc) {
            *c += d;
        }
        if self.ramping {
            if self.inc[0] != 0.0 || self.inc[2] != 0.0 {
                self.co = Coefs::new(self.cur[0], self.cur[2], self.sr);
                self.gain = self.cur[1];
                self.fbk = self.cur[3];
            }
            if self.inc[5] != 0.0 || self.inc[6] != 0.0 {
                self.dry = self.cur[5];
                self.wet = self.cur[6];
            }
            if self.inc[4] != 0.0 {
                self.size = self.cur[4];
            }
        }

        // Color: two band-passes in series. On the left the second one gets the notch
        // x − bp(x); on the right it gets bp(x).
        let x = [xl * self.gain, xr * self.gain];
        let y = self.co.run(&mut self.bq1, x);
        let d = [x[0] - y[0], x[1] - y[1]];
        let z = self.co.run(&mut self.bq2, [d[0], y[1]]);

        // Five lattice all-passes and two plain delays, all fed from the right lane.
        let w = self.w;
        let n: [i32; 7] = TAPS.map(|k| round_away(self.size * k));
        let at = |line: usize, i: i32| line * LEN + (i & MASK) as usize;
        let (v, r, b) = (&mut self.v, &mut self.r, &mut self.bufs);

        v[0] = G1.mul_add(z[1], r[0]);
        b[at(0, w)] = z[1] - v[0] * G1;
        r[0] = b[at(0, w - n[0])];

        v[1] = v[0].mul_add(G2, r[1]);
        b[at(1, w)] = v[0] - v[1] * G2;
        r[1] = b[at(1, w - n[1])];

        // The feedback tap r6 (from the previous sample) enters the third all-pass.
        let in3 = self.fbk.mul_add(r[5], v[1]);
        v[2] = in3.mul_add(G1, r[2]);
        b[at(2, w)] = in3 - v[2] * G1;
        r[2] = b[at(2, w - n[2])];

        v[3] = v[2].mul_add(G2, r[3]);
        b[at(3, w)] = v[2] - v[3] * G2;
        r[3] = b[at(3, w - n[3])];

        v[4] = v[3].mul_add(G3, r[4]);
        b[at(4, w)] = v[3] - v[4] * G3;
        r[4] = b[at(4, w - n[4])];

        b[at(5, w)] = v[4];
        r[5] = b[at(5, w - n[5])];

        b[at(6, w)] = v[3];
        let r7 = b[at(6, w - n[6])];

        let e = RESIDUE * (d[0] - z[0]);
        let wl = (r[5] + v[4]).mul_add(0.5, e);
        let wr = e + r7;
        self.w = (w + 1) & MASK;
        (self.dry * xl + self.wet * wl, self.dry * xr + self.wet * wr)
    }
}

/// The Reflex effect: the parameters (`color`, `smooth`, `size`, `mix`) go to the core as
/// events.
pub struct Reflex {
    core: Core,
    started: bool,
}

impl Reflex {
    pub fn new(sr: f64) -> Self {
        Reflex { core: Core::new(sr as f32), started: false }
    }
}

impl Effect for Reflex {
    fn set(&mut self, params: &[(String, f64)]) {
        let new = [
            param(params, "color", 0.5) as f32,
            param(params, "smooth", 0.75) as f32,
            param(params, "size", 0.25) as f32,
            param(params, "mix", 0.5) as f32,
        ];
        let c = &mut self.core;
        if !self.started {
            // The processor resumes with the stored values (§2).
            self.started = true;
            c.input = new;
            c.resume();
            return;
        }
        // Maschine sends an event only for a value that changed; each restarts the glide.
        for (p, v) in new.into_iter().enumerate() {
            if v != c.input[p] {
                c.input[p] = v;
                c.event(p);
            }
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

    fn p(color: f64, smooth: f64, size: f64, mix: f64) -> Vec<(String, f64)> {
        vec![("color".into(), color), ("smooth".into(), smooth), ("size".into(), size), ("mix".into(), mix)]
    }

    fn noise(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i * 7919 + 13) % 1000) as f32 / 1000.0 - 0.5).collect()
    }

    fn run(fx: &mut Reflex, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut a, mut b) = (l.to_vec(), r.to_vec());
        for (x, y) in a.chunks_mut(512).zip(b.chunks_mut(512)) {
            fx.process(x, y);
        }
        (a, b)
    }

    fn energy(x: &[f32]) -> f32 {
        x.iter().map(|v| v * v).sum()
    }

    #[test]
    fn targets_follow_the_tables() {
        assert_eq!(smooth_targets(0.0).1, FBK[0]); // through the word before the table
        assert_eq!(smooth_targets(1.0).1, FBK[127]);
        assert_eq!(mix_targets(0.0), (1.0, 0.0));
        assert_eq!(mix_targets(1.0), (0.0, WET));
        // Color sweeps the band-pass from MIDI note 30 to 120 (46 Hz … 8.4 kHz).
        let (f0, g0) = color_targets(0.0);
        let (f1, g1) = color_targets(1.0);
        assert!((f0 - 46.25).abs() < 0.1 && (f1 - 8372.0).abs() < 5.0, "{f0} {f1}");
        assert!(g0 == g1 / 1.5 || (g1 / g0 - 1.5).abs() < 1e-3, "{g0} {g1}");
        assert!((size_target(1.0, 48_000.0) - 48.0).abs() < 1e-3);
    }

    #[test]
    fn defaults_add_reflections() {
        let mut fx = Reflex::new(44_100.0);
        fx.set(&[]);
        let x = noise(44_100);
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        assert!(l.iter().zip(&x).any(|(a, b)| a != b));
        assert!(l.iter().zip(&r).any(|(a, b)| a != b));
    }

    #[test]
    fn dry_mix_is_the_identity() {
        let mut fx = Reflex::new(48_000.0);
        fx.set(&p(0.3, 1.0, 1.0, 0.0));
        let x = noise(20_000);
        let (l, r) = run(&mut fx, &x, &x);
        assert_eq!(l, x);
        assert_eq!(r, x);
    }

    #[test]
    fn the_right_input_feeds_the_room() {
        // A sine at the band-pass centre (Color 50 %: MIDI note 81.75, 920 Hz).
        let x: Vec<f32> = (0..30_000).map(|i| (i as f32 * 920.0 * TWO_PI / 44_100.0).sin() * 0.5).collect();
        let z = vec![0.0; x.len()];
        let mut a = Reflex::new(44_100.0);
        a.set(&p(0.5, 0.75, 0.5, 1.0));
        let (l, r) = run(&mut a, &z, &x);
        let wet = energy(&l) + energy(&r);
        // The left input only leaks in through the notch residue, which is ~0 here.
        let mut b = Reflex::new(44_100.0);
        b.set(&p(0.5, 0.75, 0.5, 1.0));
        let (l, r) = run(&mut b, &x, &z);
        let residue = energy(&l) + energy(&r);
        assert!(wet > 100.0 * residue && residue > 0.0, "{wet} {residue}");
        // Silence stays silent.
        let mut c = Reflex::new(44_100.0);
        c.set(&p(1.0, 1.0, 1.0, 1.0));
        let (l, r) = run(&mut c, &z, &z);
        assert!(l.iter().chain(&r).all(|v| *v == 0.0));
    }

    #[test]
    fn impulse_tail_decays() {
        let mut fx = Reflex::new(96_000.0);
        fx.set(&p(1.0, 1.0, 1.0, 1.0));
        let mut x = vec![0.0; 96_000 * 4];
        x[0] = 1.0;
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 4.0));
        assert!(energy(&r[..96_000]) > 1e-3);
        // Longest loop and most feedback: still −70 dB after three seconds.
        let tail = &r[96_000 * 3..];
        assert!(tail.iter().all(|v| v.abs() < 3e-4), "{:?}", tail.iter().fold(0f32, |m, v| m.max(v.abs())));
    }

    #[test]
    fn set_keeps_state_and_glides() {
        let x = noise(40_000);
        let mut a = Reflex::new(44_100.0);
        a.set(&p(0.4, 0.8, 0.6, 0.5));
        let (l1, _) = run(&mut a, &x[..10_000], &x[..10_000]);
        // Re-sending the same values changes nothing: the delay lines go on.
        a.set(&p(0.4, 0.8, 0.6, 0.5));
        let (l2, _) = run(&mut a, &x[10_000..], &x[10_000..]);
        let mut b = Reflex::new(44_100.0);
        b.set(&p(0.4, 0.8, 0.6, 0.5));
        let (lb, _) = run(&mut b, &x, &x);
        assert_eq!([l1, l2].concat(), lb);

        // Mix 0 → 1 glides over 7·128 samples (20 ms) instead of jumping.
        let mut c = Reflex::new(44_100.0);
        c.set(&p(0.4, 0.8, 0.6, 0.0));
        run(&mut c, &x[..1000], &x[..1000]);
        c.set(&p(0.4, 0.8, 0.6, 1.0));
        run(&mut c, &x[..500], &x[..500]);
        assert!(c.core.dry > 0.1 && c.core.dry < 1.0, "{}", c.core.dry);
        run(&mut c, &x[..1000], &x[..1000]);
        assert!(!c.core.ramping && c.core.dry.abs() < 1e-3, "{}", c.core.dry);
    }
}
