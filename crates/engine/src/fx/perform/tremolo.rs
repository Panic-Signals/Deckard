//! Tremolo (docs/fx-perform-tremolo.md): the level swung around unity by a sine (Trem)
//! and/or the pitch (Vibr: a swaying varispeed delay, or a frequency shifter), blended by
//! Trem/Vibr; Rate and Depth each between their Min and Max by the strip; Stereo offsets
//! the right side's phase and drives a soft auto-pan; Attk/Rel shapes how the strip's
//! touch fades it in and out. It runs whether engaged or not (the touch is its own
//! envelope); a change of Vibr Mode fades the effect out and back in (15 ms).
//!
//! Written from the description in the doc, which says which operations are fused and
//! in what precision: they are kept so here.

use super::{Ctx, Mode, Params};

const TWO_PI: f32 = 6.283_185_5;
/// `−ln 100`-ish: the followers' time constant (−4.5 over the time, in samples).
const FOLLOW: f32 = -4.5;

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// 10^x in `f64`: the system library's `__exp10` where there is one.
#[cfg(target_os = "macos")]
fn exp10(x: f64) -> f64 {
    unsafe extern "C" {
        fn __exp10(x: f64) -> f64;
    }
    // SAFETY: a pure libm function.
    unsafe { __exp10(x) }
}

#[cfg(not(target_os = "macos"))]
fn exp10(x: f64) -> f64 {
    10f64.powf(x)
}

/// `sin x ≈ x·poly(x²)` on −π..π, evaluated with fused steps.
fn sine(x: f32) -> f32 {
    let x2 = x * x;
    let c = f32::from_bits;
    let mut p = x2.mul_add(c(0xb2cd_4ca8), c(0x3638_b94f));
    p = p.mul_add(x2, c(0xb950_0c03));
    p = p.mul_add(x2, c(0x3c08_8885));
    p = p.mul_add(x2, c(0xbe2a_aac1));
    x * p.mul_add(x2, 1.0)
}

/// `q` (cycles) into 0..1 as the core wraps it, through `f64`.
fn wrap(q: f32) -> f32 {
    let d = q as f64;
    let k = ((d + 10.0) as i32) as f32;
    (d - ((k as f64) + -10.0)) as f32
}

/// A linear ramp to its target over a fixed number of samples.
#[derive(Debug, Clone, Copy)]
struct Glide {
    cur: f32,
    target: f32,
    inc: f32,
    steps: f32,
    count: i32,
    nsteps: i32,
    active: bool,
}

impl Glide {
    fn new(sr: f32, ms: f32) -> Glide {
        let steps = sr * 0.001 * ms;
        Glide { cur: 0.0, target: 0.0, inc: 0.0, steps, count: 0, nsteps: steps as i32, active: false }
    }

    fn to(&mut self, t: f32) {
        self.target = t;
        if self.cur != t {
            self.inc = (t - self.cur) / self.steps;
            self.count = self.nsteps;
            self.active = true;
        }
    }

    fn land(&mut self) {
        if self.active {
            self.cur = self.target;
            self.inc = 0.0;
            self.count = 0;
            self.active = false;
        }
    }

    fn step(&mut self) {
        if (self.target - self.cur).abs() <= 0.0 {
            self.land();
        } else {
            self.count -= 1;
            if self.count <= 0 {
                self.land();
            } else {
                self.cur += self.inc;
            }
        }
    }
}

/// One first-order allpass stage `(k + z⁻ᴰ)/(1 + k·z⁻ᴰ)` on four lanes.
#[derive(Debug, Clone, Copy)]
struct Stage {
    buf: [[f32; 4]; 8],
    w: usize,
    d: usize,
    k: [f32; 4],
}

impl Stage {
    fn tick(&mut self, x: [f32; 4]) -> [f32; 4] {
        let old = self.buf[(self.w + 8 + 1 - self.d) & 7];
        let mut keep = [0.0; 4];
        let mut y = [0.0; 4];
        for i in 0..4 {
            let t = self.k[i] * (x[i] - old[i]);
            keep[i] = x[i] + t;
            y[i] = old[i] + t;
        }
        self.w = (self.w + 1) & 7;
        self.buf[self.w] = keep;
        y
    }
}

/// The Hilbert pair's coefficients (lane a, lane b), as Maschine computes them.
const HILBERT: [(u32, u32); 6] = [
    (0x3d04_7cd2, 0x3eb7_6271),
    (0x3f1d_db76, 0x3f4a_7446),
    (0x3f66_2b90, 0x3f78_4ff1),
    (0xbf12_9682, 0xbf42_4d26),
    (0xbf5f_0faa, 0xbf6f_3ac2),
    (0xbf78_3b8e, 0xbf7d_bf3b),
];

pub struct Tremolo {
    sr: f32,
    inv_sr: f64,
    /// The raw parameters by core index (§1).
    p: [f32; 10],
    vib_w: f32,
    trem_w: f32,
    /// Freq Shift (else Varispeed).
    fs: bool,
    /// Until the first sample, settings land at once.
    suspended: bool,
    first: bool,
    rate: Glide,
    trem: Glide,
    vib: Glide,
    pan: Glide,
    fade: Glide,
    attack: Glide,
    release: Glide,
    stereo: Glide,
    engage: Glide,
    phase: f64,
    e1: f32,
    e2: f32,
    lines: [Vec<f32>; 2],
    w: usize,
    stages: [Stage; 7],
    re: f64,
    im: f64,
}

impl Tremolo {
    pub fn new(sr: f32) -> Tremolo {
        let g = |ms| Glide::new(sr, ms);
        let n = ((sr * 0.15) as i64).max(2) as usize;
        let d = (((sr + 1.0) / 44_100.0) as i32).clamp(1, 8) as usize;
        let mut stages = [Stage { buf: [[0.0; 4]; 8], w: 0, d, k: [-0.3; 4] }; 7];
        for (st, (a, b)) in stages.iter_mut().zip(HILBERT) {
            let (a, b) = (f32::from_bits(a), f32::from_bits(b));
            st.k = [a, b, a, b];
        }
        Tremolo {
            sr,
            inv_sr: 1.0 / sr as f64,
            p: [0.0; 10],
            vib_w: 0.0,
            trem_w: 0.0,
            fs: false,
            suspended: true,
            first: true,
            rate: g(200.0),
            trem: g(200.0),
            vib: g(200.0),
            pan: g(200.0),
            fade: g(15.0),
            attack: g(200.0),
            release: g(200.0),
            stereo: g(200.0),
            engage: g(200.0),
            phase: 0.0,
            e1: 0.0,
            e2: 0.0,
            lines: [vec![0.0; n.next_power_of_two()], vec![0.0; n.next_power_of_two()]],
            w: 0,
            stages,
            re: 1.0,
            im: 0.0,
        }
    }

    fn glides(&mut self) -> [&mut Glide; 9] {
        [
            &mut self.rate,
            &mut self.trem,
            &mut self.vib,
            &mut self.pan,
            &mut self.fade,
            &mut self.attack,
            &mut self.release,
            &mut self.stereo,
            &mut self.engage,
        ]
    }

    /// The core's `setParameter(i, x)` (§1).
    fn param(&mut self, i: usize, x: f32) {
        self.p[i] = x;
        match i {
            1 => self.engage.to(if x > 0.5 { 1.0 } else { 0.0 }),
            2 => {
                let time = |a: f32, scale: u32| {
                    let db = (a as f64).mul_add(35.0, 40.0) as f32;
                    exp10(db as f64 * 0.05) as f32 * f32::from_bits(scale)
                };
                let a = clamp01((-x.abs()).mul_add(0.5, 1.5) * x);
                self.attack.to(time(a, 0x3a83_126f));
                let y = x.abs().mul_add(f32::from_bits(0x3e4c_cccd), f32::from_bits(0x3f4c_cccd)) * x;
                let r = clamp01(y * (-y.abs()).mul_add(0.5, 1.5));
                self.release.to(time(r, 0x39d1_b718));
            }
            4 => {
                self.vib_w = clamp01(x + x);
                let y = 1.0 - x;
                self.trem_w = clamp01(y + y);
                self.param(7, self.p[7]);
                return;
            }
            5 => {
                if (x > 0.0) != self.fs {
                    self.fade.to(0.0);
                }
            }
            _ => self.ranges(),
        }
        if self.suspended {
            for g in self.glides() {
                g.land();
            }
        }
    }

    /// Rate, depths, pan and stereo from the strip and the ranges (§1, default).
    fn ranges(&mut self) {
        let s = clamp01(self.p[0]) as f64;
        let (d0, d1) = (self.p[6], self.p[7]);
        let lerp = |a: f32, b: f32| (s.mul_add((b - a) as f64, a as f64)) as f32;
        let r = |x: f32| (clamp01(x) as f64).mul_add(f64::from_bits(0x4041_70fb_e000_0000), f64::from_bits(0xc038_6056_4000_0000)) as f32;
        let oct = lerp(r(self.p[8]), r(self.p[9])) as f64;
        self.rate.to(oct.mul_add(f64::from_bits(0x3fb5_5555_5555_5555), f64::from_bits(0x4008_4039_7fa8_81bf)).exp2() as f32);
        let g = |d: f32| d * (-d.abs()).mul_add(f32::from_bits(0xbf19_999a), f32::from_bits(0x3ecc_cccc));
        self.vib.to(self.vib_w * lerp(g(d0), g(d1)));
        let h = |d: f32| d * (-d.abs()).mul_add(f32::from_bits(0xbf33_3333), f32::from_bits(0x3e99_999a));
        self.trem.to(self.trem_w * lerp(h(d0), h(d1)));
        self.pan.to(self.p[3] * (1.0 - self.trem_w) * lerp(d0, d1));
        self.stereo.to(self.p[3]);
    }

    /// The mode switch, run while the fade is at 0 (§4).
    fn switch(&mut self) {
        let fs = self.p[5] > 0.5;
        if fs != self.fs {
            self.re = 1.0;
            self.im = 0.0;
            for st in &mut self.stages {
                st.buf = [[0.0; 4]; 8];
                st.w = 0;
            }
            for l in &mut self.lines {
                l.fill(0.0);
            }
        }
        self.fs = fs;
        self.fade.to(1.0);
    }

    /// The followers' coefficient for a time in seconds.
    fn coef(&self, t: f32) -> f32 {
        if t > 0.0 { 1.0 - (FOLLOW / (t * self.sr)).exp() } else { 1.0 }
    }

    fn varispeed(&mut self, x: [f32; 2], vib: f32, rate: f32, sin_l: f32) -> [f32; 2] {
        let mask = self.lines[0].len() - 1;
        for (l, v) in self.lines.iter_mut().zip(x) {
            l[self.w] = v;
        }
        self.w = (self.w + 1) & mask;
        if self.e2 <= 0.0 {
            return x;
        }
        let sr = self.sr;
        let most = sr.mul_add(f32::from_bits(0x3e19_999a), -3.0);
        let sway = ((sin_l as f64).mul_add(0.5, 0.5)) as f32;
        let mut d = ((vib * f32::from_bits(0x3e4c_cccd)) * self.e2 / rate) * sway * sr;
        if d < 3.0 {
            d = 3.0;
        }
        let d = if most < d { most } else { d };
        let k = ((-d) as i32) - (d >= 0.0) as i32;
        let t = (-d) as f64 - k as f64;
        let (t2, base) = (t * t, self.w as i64 + k as i64);
        let t3 = t * t2;
        let at = |i: i64| (base + i) as usize & mask;
        let mut out = [0.0; 2];
        for (o, l) in out.iter_mut().zip(&self.lines) {
            let (p0, p1, p2, p3) = (l[at(-1)], l[at(0)], l[at(1)], l[at(2)]);
            let c3 = p3.mul_add(0.5, p2.mul_add(-1.5, p0.mul_add(-0.5, p1 * 1.5)));
            let c2 = p3.mul_add(-0.5, p2.mul_add(2.0, p1.mul_add(-2.5, p0)));
            let c1 = p0.mul_add(-0.5, p2 * 0.5);
            let y = (c1 as f64).mul_add(t, (c3 as f64).mul_add(t3, t2 * c2 as f64));
            *o = (y + p1 as f64) as f32;
        }
        out
    }

    fn freq_shift(&mut self, x: [f32; 2], vib: f32, sin_l: f32) -> [f32; 2] {
        let hz = sin_l * ((vib * self.e2) * 90.0);
        let w = (hz as f64 * std::f64::consts::TAU) / self.sr as f64;
        let (s, c) = w.sin_cos();
        let mut v = [x[0], x[0], x[1], x[1]];
        let mut a = v;
        for (i, st) in self.stages.iter_mut().enumerate() {
            if i == 6 {
                a = v;
            }
            v = st.tick(v);
        }
        let re = c.mul_add(self.re, -(s * self.im)) + 0.0;
        let im = s.mul_add(self.re, c * self.im);
        (self.re, self.im) = (re, im);
        let (re, im) = (re as f32, im as f32);
        [im * a[0] + re * v[1], im * a[2] + re * v[3]]
    }
}

impl Mode for Tremolo {
    fn set(&mut self, p: &Params) {
        let v = [
            p.f("touchstrip_pos"),
            p.f("touchstrip_on_off"),
            p.f("tremolo_attack_release"),
            p.f("tremolo_stereo"),
            p.f("tremolo_trem_vibr"),
            p.f("tremolo_vibr_mode"),
            p.f("tremolo_depth_min"),
            p.f("tremolo_depth_max"),
            p.f("tremolo_rate_min"),
            p.f("tremolo_rate_max"),
        ];
        // Only what changed reaches the core (a repeated target would restart its ramp).
        for (i, x) in v.into_iter().enumerate() {
            if self.first || x.to_bits() != self.p[i].to_bits() {
                self.param(i, x);
            }
        }
        self.first = false;
    }

    fn frame(&mut self, l: f32, r: f32, _c: &Ctx) -> (f32, f32) {
        self.suspended = false;
        for g in self.glides() {
            g.step();
        }
        let fade = self.fade.cur;
        if fade == 0.0 {
            self.switch();
            if !self.fs {
                // Varispeed's depth comes in from nothing.
                let v = &mut self.vib;
                (v.cur, v.inc, v.count) = (0.0, 0.0, 0);
                if v.target != 0.0 {
                    v.inc = v.target / v.steps;
                    v.count = v.nsteps;
                    v.active = true;
                }
            }
        }
        let (stereo, rate) = (self.stereo.cur, self.rate.cur);
        let u = (rate as f64).mul_add(self.inv_sr, self.phase);
        self.phase = u - ((((u + 10.0) as i32) as f64) + -10.0);
        let (trem, vib, touch) = (self.trem.cur, self.vib.cur, self.engage.cur.abs());
        let (ca, cr) = (self.coef(self.attack.cur), self.coef(self.release.cur));
        self.e1 = (touch - self.e1).mul_add(if touch > self.e1 { ca } else { cr }, self.e1);
        let e1 = self.e1.abs();
        self.e2 = (e1 - self.e2).mul_add(if e1 > self.e2 { ca } else { cr }, self.e2);
        let sin_l = sine((self.phase - 0.5) as f32 * TWO_PI);
        let [vl, vr] = if self.fs { self.freq_shift([l, r], vib, sin_l) } else { self.varispeed([l, r], vib, rate, sin_l) };
        let u = u as f32;
        let sin_r = sine((wrap(stereo * 0.5 + u) + -0.5) * TWO_PI);
        let sin_p = sine((wrap(u + 0.5) + -0.5) * TWO_PI);
        let td = trem * self.e2;
        let (mut ol, mut or) = (td.mul_add(sin_l, 1.0) * vl, td.mul_add(sin_r, 1.0) * vr);
        let p = sin_p * (self.e2 * self.pan.cur);
        let gain = |g: f32| (g * f32::from_bits(0x3eaa_aaab)) * (4.0 - g);
        ol *= gain(1.0 - p);
        or *= gain(p + 1.0);
        let dry = 1.0 - fade;
        (l.mul_add(dry, fade * ol), r.mul_add(dry, fade * or))
    }
}

#[cfg(test)]
mod tests {
    //! Null tests against Maschine 3's renders (docs/fx-perform-tremolo.md §8), made with
    //! `maschine-ref make … --perform 6 [field=value…]` and `render --input noise
    //! --seconds 2` (48 kHz, 1 s preroll) into `out/perform-tremolo/` (not committed);
    //! skipped where those files aren't there.

    use crate::fx::Effect;
    use crate::fx::perform::PerformFx;
    use std::path::PathBuf;

    fn dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/perform-tremolo")
    }

    fn read(name: &str) -> Option<(u32, Vec<f32>, Vec<f32>)> {
        let mut r = hound::WavReader::open(dir().join(name)).ok()?;
        let spec = r.spec();
        let s: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => r.samples::<f32>().map(Result::unwrap).collect(),
            hound::SampleFormat::Int => {
                let k = (1u64 << (spec.bits_per_sample - 1)) as f32;
                r.samples::<i32>().map(|x| x.unwrap() as f32 / k).collect()
            }
        };
        let n = spec.channels as usize;
        Some((spec.sample_rate, s.iter().step_by(n).copied().collect(), s.iter().skip(1).step_by(n).copied().collect()))
    }

    /// DECKARD's Perform FX over case `name`'s input, set as `make` set it; the residual
    /// against Maschine's output (dB RMS, the worse side).
    fn null(name: &str, input: &str, set: &[(&str, f64)]) -> Option<f64> {
        let (sr, il, ir) = read(&format!("{input}-in.wav"))?;
        let (_, rl, rr) = read(&format!("{name}.wav"))?;
        let mut p: Vec<(String, f64)> = deckard_model::PERFORM_FX.1.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        let mut put = |k: &str, v: f64| match p.iter_mut().find(|(n, _)| n == k) {
            Some(x) => x.1 = v,
            None => p.push((k.to_string(), v)),
        };
        put("mode", 6.0);
        put("touchstrip_on_off", 1.0);
        for (k, v) in set {
            put(k, *v);
        }
        let mut fx = PerformFx::new(sr as f64);
        fx.set(&p);
        fx.set_tempo(120.0);
        // maschine-ref's 1 s preroll of processed silence.
        let mut pre = sr as usize;
        while pre > 0 {
            let n = pre.min(512);
            let (mut a, mut b) = (vec![0.0; n], vec![0.0; n]);
            fx.process(&mut a, &mut b);
            pre -= n;
        }
        let (mut l, mut r) = (il.clone(), ir.clone());
        for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            fx.process(a, b);
        }
        let db = |x: &[f32], y: &[f32]| {
            let n = x.len().min(y.len());
            let e = x[..n].iter().zip(&y[..n]).map(|(a, b)| ((a - b) as f64).powi(2)).sum::<f64>() / n as f64;
            if e == 0.0 { -240.0 } else { 10.0 * e.log10() }
        };
        Some(db(&l, &rl).max(db(&r, &rr)))
    }

    fn check(name: &str, input: &str, set: &[(&str, f64)]) {
        let Some(db) = null(name, input, set) else { return };
        eprintln!("tremolo null {name}: {db:.1} dB");
        assert!(db < -120.0, "{name}: residual {db:.1} dB");
    }

    #[test]
    fn nulls_at_the_defaults() {
        check("def", "def", &[]);
    }

    #[test]
    fn nulls_as_varispeed_vibrato() {
        check("vib", "vib", &[("tremolo_trem_vibr", 1.0)]);
        check(
            "vib2",
            "vib2",
            &[("tremolo_trem_vibr", 0.8), ("touchstrip_pos", 0.9), ("tremolo_depth_max", 1.0), ("tremolo_rate_max", 0.9)],
        );
    }

    #[test]
    fn nulls_as_freq_shift_vibrato() {
        check("fs", "fs", &[("tremolo_trem_vibr", 1.0), ("tremolo_vibr_mode", 1.0)]);
        check("fs-sine", "fs-sine", &[("tremolo_trem_vibr", 1.0), ("tremolo_vibr_mode", 1.0)]);
    }

    /// 44.1 kHz and 96 kHz (where the Freq Shift's allpasses run on every second sample).
    #[test]
    fn nulls_at_other_rates() {
        for sr in [44_100, 96_000] {
            let fs = [("tremolo_trem_vibr", 1.0), ("tremolo_vibr_mode", 1.0)];
            check(&format!("fs-{sr}"), &format!("fs-{sr}"), &fs);
            let vib2 = [("tremolo_trem_vibr", 0.8), ("touchstrip_pos", 0.9), ("tremolo_depth_max", 1.0), ("tremolo_rate_max", 0.9)];
            check(&format!("vib2-{sr}"), &format!("vib2-{sr}"), &vib2);
            let mix = [("tremolo_trem_vibr", 0.6), ("tremolo_stereo", 1.0), ("touchstrip_pos", 0.2), ("tremolo_attack_release", 0.0)];
            check(&format!("mix-{sr}"), &format!("mix-{sr}"), &mix);
        }
    }

    #[test]
    fn nulls_disengaged() {
        check("off", "off", &[("touchstrip_on_off", 0.0)]);
        // Freq Shift isn't transparent disengaged: an allpass copy of the input.
        check("fsoff", "fsoff", &[("touchstrip_on_off", 0.0), ("tremolo_vibr_mode", 1.0)]);
    }

    #[test]
    fn nulls_with_stereo_pan_and_extremes() {
        check(
            "mix",
            "mix",
            &[("tremolo_trem_vibr", 0.6), ("tremolo_stereo", 1.0), ("touchstrip_pos", 0.2), ("tremolo_attack_release", 0.0)],
        );
        check(
            "fast",
            "fast",
            &[
                ("tremolo_trem_vibr", 0.0),
                ("touchstrip_pos", 1.0),
                ("tremolo_rate_max", 1.0),
                ("tremolo_depth_max", 1.0),
                ("tremolo_attack_release", 1.0),
                ("tremolo_stereo", 0.7),
            ],
        );
    }

    /// Let go, the tremolo dies away with Attk/Rel's release (two followers) and the
    /// input comes back.
    #[test]
    fn releasing_the_strip_lets_it_die_away() {
        let mut p: Vec<(String, f64)> = deckard_model::PERFORM_FX.1.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        let put = |p: &mut Vec<(String, f64)>, k: &str, v: f64| p.iter_mut().find(|(n, _)| n == k).unwrap().1 = v;
        put(&mut p, "mode", 6.0);
        put(&mut p, "touchstrip_on_off", 1.0);
        put(&mut p, "tremolo_trem_vibr", 0.0);
        put(&mut p, "tremolo_depth_max", 1.0);
        let mut fx = PerformFx::new(48_000.0);
        fx.set(&p);
        let x = vec![0.5f32; 48_000];
        let (mut l, mut r) = (x.clone(), x.clone());
        fx.process(&mut l, &mut r);
        let swing = |v: &[f32]| v.iter().fold(0.0f32, |m, a| m.max((a - 0.5).abs()));
        assert!(swing(&l[24_000..]) > 0.1, "engaged: {}", swing(&l[24_000..]));
        put(&mut p, "touchstrip_on_off", 0.0);
        fx.set(&p);
        for _ in 0..3 {
            let (mut l, mut r) = (x.clone(), x.clone());
            fx.process(&mut l, &mut r);
        }
        let (mut l, mut r) = (x.clone(), x.clone());
        fx.process(&mut l, &mut r);
        assert!(swing(&l) < 1e-4, "released: {}", swing(&l));
    }

    /// Disengaged in Varispeed, the input comes through untouched (after the 15 ms
    /// fade-in every Tremolo starts with).
    #[test]
    fn disengaged_varispeed_is_transparent() {
        let mut p: Vec<(String, f64)> = deckard_model::PERFORM_FX.1.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        p.iter_mut().find(|(n, _)| n == "mode").unwrap().1 = 6.0;
        let mut fx = PerformFx::new(48_000.0);
        fx.set(&p);
        let x: Vec<f32> = (0..9600).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let (mut l, mut r) = (x.clone(), x.clone());
        fx.process(&mut l, &mut r);
        assert_eq!(&l[1000..], &x[1000..]);
        assert_eq!(&r[1000..], &x[1000..]);
        // The fade-in mixes the input with itself: within rounding.
        let worst = l.iter().zip(&x).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(worst < 1e-6, "{worst}");
    }
}
