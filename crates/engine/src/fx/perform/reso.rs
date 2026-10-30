//! Reso Echo (docs/fx-perform-reso-echo.md): the input, and the echo's feedback, run
//! through four resonant band passes (two per side, panned by the strip) whose
//! frequencies the strip sets; their sum, high-passed at 60 Hz, driven into a limiter
//! (Pump) and a saturator, feeds a stereo delay (the right side 0.8 of the left's time)
//! read with cubic interpolation. Engaged, the input feeds it at full feedback; released,
//! the feedback falls to 0.15 of it over a time Time and Feedback set, so the tail rings
//! out and dies. Mix modes: Mix (the delay and the resonators, the dry at 0.7 while
//! engaged), Insert (the dry through a fifth band pass too) and Wet only.

use super::kit::{Glide, Tpt, exp10, tan5};
use super::{Ctx, Mode, Params};

const PI_F: f32 = std::f32::consts::PI;
/// ln(0.001): a −60 dB time constant.
const LN_MILLI: f64 = f64::from_bits(0xc01b_a18a_998f_ffa0);
/// dB → exp-table steps (60 / (20·log10 2)).
const K_DB: f32 = f32::from_bits(0x411f_73da);

/// Ten state-variable band passes: a pair per frequency (the insert filter, then four
/// resonators).
#[derive(Default)]
struct Bank {
    ic1: [f32; 10],
    ic2: [f32; 10],
    a1: [f32; 10],
    a2: [f32; 10],
    a3: [f32; 10],
    v1: [f32; 10],
}

impl Bank {
    /// Coefficients for frequencies `f` and damping `k`.
    fn tune(&mut self, f: &[f32; 10], k: f32, inv_sr: f32) {
        for (i, f) in f.iter().enumerate() {
            let g = tan5(f * PI_F * inv_sr);
            self.a1[i] = 1.0 / (k + g).mul_add(g, 1.0);
            self.a2[i] = g * self.a1[i];
            self.a3[i] = g * self.a2[i];
        }
    }

    fn tick(&mut self, x: &[f32; 10]) {
        for (i, x) in x.iter().enumerate() {
            let (ic1, ic2) = (self.ic1[i], self.ic2[i]);
            let v3 = x - ic2;
            let v1 = self.a1[i].mul_add(ic1, v3 * self.a2[i]);
            let v2 = ic2 + self.a2[i].mul_add(ic1, v3 * self.a3[i]);
            self.v1[i] = v1;
            self.ic1[i] = v1 * 2.0 - ic1;
            self.ic2[i] = v2 * 2.0 - ic2;
        }
    }
}

/// The 60 Hz high pass after the resonators (two channels).
#[derive(Default)]
struct HighPass {
    ic1: [f32; 2],
    ic2: [f32; 2],
    a1: f32,
    a2: f32,
    a3: f32,
}

impl HighPass {
    fn new(inv_sr: f32) -> HighPass {
        let g = tan5(inv_sr * f32::from_bits(0x433c_7edd));
        let a1 = 1.0 / g.mul_add(g + 2.0, 1.0);
        let a2 = g * a1;
        HighPass { a1, a2, a3: g * a2, ..HighPass::default() }
    }

    fn tick(&mut self, x: [f32; 2]) -> [f32; 2] {
        let mut out = [0.0; 2];
        for c in 0..2 {
            let (ic1, ic2) = (self.ic1[c], self.ic2[c]);
            let v3 = x[c] - ic2;
            let v2 = ic2 + ic1.mul_add(self.a2, self.a3 * v3);
            let v1 = ic1.mul_add(self.a1, v3 * self.a2);
            out[c] = (-v1).mul_add(2.0, x[c]) - v2;
            self.ic1[c] = 2.0f32.mul_add(v1, -ic1);
            self.ic2[c] = 2.0f32.mul_add(v2, -ic2);
        }
        out
    }
}

/// The limiter Reso Echo drives (Threshold 0.9, Release 0.01, Ceiling 0.94): a peak
/// follower in dB with two release paths, the gain from the exp-table.
struct Limiter {
    pre: f32,
    post: f32,
    env: f32,
    slow: f32,
    c_up: f32,
    c_down: f32,
    c_rise: f32,
    c_fall: f32,
}

impl Limiter {
    fn new(sr: f32) -> Limiter {
        let e = crate::fx::exp_e;
        let rel = (0.01f32 * 6.0).max(0.001);
        Limiter {
            pre: e(((1.0 - 0.9f32) * 40.0).mul_add(K_DB, 1200.0)),
            post: e(((1.0 - f32::from_bits(0x3f70_a3d7)) * -40.0).mul_add(K_DB, 1200.0)),
            env: 0.0,
            slow: 0.0,
            c_up: 1.0 - 1.0 / (0.005 * sr),
            c_down: 1.0 - 1.0 / (rel * sr),
            c_rise: 1.0 / (-15.0 * sr) + 1.0,
            c_fall: 1.0 / (-3.0 * sr) + 1.0,
        }
    }

    fn tick(&mut self, x: [f32; 2]) -> [f32; 2] {
        let a = x[0].mul_add(self.pre, 1e-12).abs();
        let b = x[1].mul_add(self.pre, 1e-12).abs();
        let peak = if a < b { b } else { a };
        let db = peak.ln() * f32::from_bits(0x410a_f968);
        let c = if self.env < db { self.c_up } else { self.c_down };
        self.env = c.mul_add(self.env - db, db);
        let c = if self.slow < db { self.c_rise } else { self.c_fall };
        self.slow = c.mul_add(self.slow - db, db);
        let m = if self.env < self.slow { self.slow } else { self.env };
        let g = crate::fx::exp_e((0.0 - m).min(0.0).mul_add(K_DB, 1200.0));
        x.map(|v| (self.post * (g * v)).clamp(-2.5, 2.5))
    }
}

/// The glides, by what they move.
const POS: usize = 0;
const FREQ: usize = 1;
const RES: usize = 6;
const FEEDBACK: usize = 7;
const ON: usize = 8;
const TIME: usize = 9;
const DRIVE: usize = 10;
const SAT_OUT: usize = 11;
const PUMP: usize = 12;
const INSERT: usize = 13;
const SHAPE: usize = 14;
const DRY: usize = 15;
const WET: usize = 16;

pub struct ResoEcho {
    sr: f32,
    inv_sr: f32,
    /// The settings as last set, by Maschine's index (0 strip, 1 engage, 2 Resonance,
    /// 3 Feedback, 4 Time, 5 Saturation, 6 Pump, 7 Mix Mode, 8 Wet Level).
    raw: [f32; 9],
    first: bool,
    glides: [Glide; 17],
    /// Resonance and Time as mapped (0.6–0.86; dB of ms), which Feedback reads.
    res: f32,
    time_db: f32,
    engaged: bool,
    mix: u8,
    bank: Bank,
    hp: HighPass,
    limiter: Limiter,
    on: Tpt,
    delay: Tpt,
    /// The feedback's level after a release (1 → 0) and its fall per sample.
    env: f64,
    fall: f64,
    buf: [Vec<f32>; 2],
    w: usize,
    fb: [f32; 2],
}

impl ResoEcho {
    pub fn new(sr: f32) -> ResoEcho {
        let inv_sr = 1.0 / sr;
        let len = (sr.mul_add(0.5, 5.0) as usize).next_power_of_two();
        ResoEcho {
            sr,
            inv_sr,
            raw: [f32::NAN; 9],
            first: true,
            glides: [Glide::new(sr); 17],
            res: 0.0,
            time_db: 0.0,
            engaged: false,
            mix: 0,
            bank: Bank::default(),
            hp: HighPass::new(inv_sr),
            limiter: Limiter::new(sr),
            on: Tpt::new(3.0, inv_sr),
            delay: Tpt::new(3.0, inv_sr),
            env: 1.0,
            fall: 1.0,
            buf: [vec![0.0; len], vec![0.0; len]],
            w: 0,
            fb: [0.0; 2],
        }
    }

    /// The feedback gain and the release's fall (Maschine's `setFeedback`).
    fn feedback(&mut self) {
        let fb = self.raw[3];
        let a = ((self.res - 0.6) * 4.0).clamp(0.0, 1.0);
        let gain = exp10(f64::from(f64::from(a).mul_add(3.0, -1.5) as f32) * 0.05) as f32;
        let p = (self.raw[0] / 0.2).clamp(0.0, 1.0);
        let pos = f64::from(p).mul_add(f64::from(-0.200_000_05f32), f64::from(1.2f32)) as f32;
        let t = ((self.time_db - 30.0) / 23.0).clamp(0.0, 1.0);
        let t = f64::from(t).mul_add(f64::from(0.099_999_964f32), f64::from(0.6f32)) as f32;
        let amt = f64::from(fb.clamp(0.0, 1.0)).mul_add(f64::from(t + -0.2), f64::from(0.2f32)) as f32;
        self.glides[FEEDBACK].set(gain * pos * amt);
        // Released, the feedback falls to 0.15 of itself over this long (−60 dB).
        let u = ((self.time_db + -38.0) / 12.0).clamp(0.0, 1.0);
        let d0 = 45.0 - f64::from(f64::from(u).mul_add(10.0, 0.0) as f32);
        let f = if fb < 0.0 { 0.0 } else { f64::from(fb).min(1.0) };
        let secs = exp10(f.mul_add(79.0 - d0, d0) * 0.05) * 0.001;
        self.fall = if secs > 0.0 { 1.0 - (LN_MILLI / (secs * f64::from(self.sr))).exp() } else { 1.0 };
    }

    /// One setting by its index (Maschine's `setParameter`).
    fn param(&mut self, i: usize, v: f32) {
        self.raw[i] = v;
        let unit = v.clamp(0.0, 1.0);
        let db = |x: f64| exp10(f64::from(x as f32) * 0.05) as f32;
        match i {
            0 => {
                // The bank's frequencies: MIDI notes along the strip.
                let c = f64::from(unit);
                let notes: [(f64, f64); 5] = [
                    (110.0, 20.0),
                    (85.0, 0.0),
                    (f64::from(76.7f32), f64::from(30.3f32)),
                    (80.0, 41.0),
                    (f64::from(80.2f32), f64::from(49.8f32)),
                ];
                for (k, (a, b)) in notes.into_iter().enumerate() {
                    let note = f64::from(c.mul_add(a, b) as f32);
                    let hz = note.mul_add(f64::from_bits(0x3fb5_5555_5555_5555), f64::from_bits(0x4008_4039_7fa8_81bf)).exp2();
                    self.glides[FREQ + k].set(hz as f32);
                }
                self.glides[POS].set(v);
                self.feedback();
            }
            1 => {
                let on = v > 0.5;
                self.glides[ON].set(if on { 1.0 } else { 0.0 });
                if !on && self.engaged {
                    self.env = 1.0;
                }
                self.engaged = on;
                self.dry();
            }
            2 => {
                self.res = f64::from(unit).mul_add(f64::from(0.26f32), f64::from(0.6f32)) as f32;
                self.glides[RES].set(self.res);
                self.feedback();
            }
            3 => self.feedback(),
            4 => {
                let x = (v.abs().mul_add(-0.5, 1.5) * v).clamp(0.0, 1.0);
                self.time_db = f64::from(x).mul_add(f64::from(30.9794f32), 23.0) as f32;
                self.glides[TIME].set(self.sr * (exp10(f64::from(self.time_db) * 0.05) * 0.001) as f32);
                self.feedback();
            }
            5 => {
                let c = f64::from(unit);
                self.glides[DRIVE].set(db(c.mul_add(14.0, -4.0)));
                self.glides[SAT_OUT].set(db(c.mul_add(-14.0, 4.0)));
            }
            6 => self.glides[PUMP].set(db(f64::from(unit).mul_add(24.0, 0.0))),
            7 => {
                if v <= 0.5 {
                    self.mix = 0;
                } else if v <= 1.5 {
                    self.mix = 1;
                } else if v <= 2.00001 {
                    self.mix = 2;
                } else {
                    return;
                }
                let (shape, insert) = match self.mix {
                    0 => (0.5, 0.0),
                    1 => (1.0, 1.0),
                    _ => (1.0, 0.0),
                };
                self.glides[SHAPE].set(shape);
                self.glides[INSERT].set(insert);
                self.dry();
            }
            8 => self.glides[WET].set(v * 1.5),
            _ => {}
        }
    }

    /// The dry signal's level: Mix 0.7 while engaged (else 1), Insert 1, Wet only 0.
    fn dry(&mut self) {
        let d = match self.mix {
            0 if self.engaged => 0.7,
            0 | 1 => 1.0,
            _ => 0.0,
        };
        self.glides[DRY].set(d);
    }

    /// At the settings at once, the states clear (Maschine's `onReset`).
    fn reset(&mut self) {
        for g in &mut self.glides {
            g.land();
        }
        self.bank = Bank::default();
        self.hp.ic1 = [0.0; 2];
        self.hp.ic2 = [0.0; 2];
        for b in &mut self.buf {
            b.fill(0.0);
        }
        self.w = 0;
        self.delay.settle(self.glides[TIME].target);
        self.on.settle(self.glides[ON].target);
        self.fb = [0.0; 2];
        self.env = 1.0;
    }

    /// Side `c` of the delay `at` (negative) samples from the write point, cubic.
    fn read(&self, c: usize, at: f32) -> f32 {
        let mask = self.buf[c].len() - 1;
        let i = at as i64 - i64::from(at <= 0.0);
        let f = f64::from(at) - i as f64;
        let base = self.w as i64 + i;
        let y = |k: i64| self.buf[c][(base + k) as usize & mask];
        let (ym1, y0, y1, y2) = (y(-1), y(0), y(1), y(2));
        let c3 = 0.5f32.mul_add(y2, (-1.5f32).mul_add(y1, (-0.5f32).mul_add(ym1, y0 * 1.5)));
        let c1 = (-0.5f32).mul_add(ym1, y1 * 0.5);
        let c2 = (-0.5f32).mul_add(y2, 2.0f32.mul_add(y1, (-2.5f32).mul_add(y0, ym1)));
        let f2 = f * f;
        let f3 = f * f2;
        let out = f.mul_add(f64::from(c1), f3.mul_add(f64::from(c3), f2 * f64::from(c2)));
        (out + f64::from(y0)) as f32
    }
}

impl Mode for ResoEcho {
    fn set(&mut self, p: &Params) {
        let vals = [
            p.f("touchstrip_pos"),
            if p.f("touchstrip_on_off") >= 0.5 { 1.0 } else { 0.0 },
            p.f("reso_resonance"),
            p.f("reso_feedback"),
            p.f("reso_time"),
            p.f("reso_saturation"),
            p.f("reso_pump"),
            p.i("reso_mix_mode") as f32,
            p.f("reso_wet_level"),
        ];
        for (i, v) in vals.into_iter().enumerate() {
            if self.first || v != self.raw[i] {
                self.param(i, v);
            }
        }
        if self.first {
            self.first = false;
            self.reset();
        }
    }

    fn frame(&mut self, l: f32, r: f32, _c: &Ctx) -> (f32, f32) {
        for g in &mut self.glides {
            g.tick();
        }
        let gl = |i: usize| self.glides[i].cur;
        let f: [f32; 10] = std::array::from_fn(|k| gl(FREQ + k / 2));
        let k = gl(RES).mul_add(-2.0, 2.0);
        self.bank.tune(&f, k, self.inv_sr);
        let e = self.on.tick(gl(ON));
        let feed = e * 1.3;
        let a = feed.mul_add(l, self.fb[0]);
        let b = feed.mul_add(r, self.fb[1]);
        self.bank.tick(&[l * 0.8, r * 0.8, a, b, a, b, a, b, a, b]);
        let band = |i: usize| k * self.bank.v1[i];
        let insert = [band(0), band(1)];
        // The resonators' pan: a triangle of the strip each, 1 ∓ w on the two sides.
        let p = gl(POS);
        let tri = |x: f32, depth: f32| {
            let u = (x - ((x + 10.5) as i32) as f32) + 10.0;
            ((u.abs() + -0.25) * 4.0) * depth
        };
        let w = [tri(p * 0.7, 0.25), tri(p.mul_add(1.3, 0.1), 0.4), tri(p.mul_add(1.3, -0.3), -0.3), tri(p.mul_add(0.1, -0.2), -0.25)];
        let third = f32::from_bits(0x3eaa_aa9f);
        let curve = |g: f32| (g * third) * (4.0 - g);
        let mut sum = [0.0f32; 2];
        for (c, s) in sum.iter_mut().enumerate() {
            let side = |i: usize| if c == 0 { 1.0 - w[i] } else { w[i] + 1.0 };
            let mut v = band(4 + c) * curve(side(1));
            v = band(2 + c).mul_add(curve(side(0)), v);
            v = band(6 + c).mul_add(curve(side(2)), v);
            v = band(8 + c).mul_add(curve(side(3)), v);
            *s = v + v;
        }
        let pump = gl(PUMP);
        let hp = self.hp.tick(sum);
        let lim = self.limiter.tick([hp[0] * pump, hp[1] * pump]);
        let un_pump = 1.0 / (pump + 1e-20);
        let y = lim.map(|v| gl(SAT_OUT) * (gl(DRIVE) * v).tanh());
        let d = self.delay.tick(gl(TIME));
        let c = if self.env < 0.0 { 1.0 } else { self.fall };
        self.env = (0.0 - self.env).mul_add(c, self.env);
        let on = if self.engaged { 1.0f32 } else { 0.0 };
        let env = self.env as f32;
        let m = if on > env { on } else { env };
        let fb_gain = f64::from(m).mul_add(f64::from(gl(FEEDBACK) + -0.15), f64::from(0.15f32)) as f32;
        let mask = self.buf[0].len() - 1;
        self.buf[0][self.w] = y[0];
        self.buf[1][self.w] = y[1];
        self.w = (self.w + 1) & mask;
        let tap = [self.read(0, -d), self.read(1, d * -0.8)];
        let shape = gl(SHAPE);
        let own = shape * (2.0 - shape);
        let echo = (1.0 - shape) * (2.0 - (1.0 - shape));
        let wet = gl(WET);
        let z = f64::from(gl(INSERT) * e);
        let dry_gain = z.mul_add(f64::from(wet - gl(DRY)), f64::from(gl(DRY))) as f32;
        let mut out = [0.0f32; 2];
        for (ch, x) in [l, r].into_iter().enumerate() {
            let wet_out = tap[ch].mul_add(echo, y[ch] * own) * wet;
            self.fb[ch] = (tap[ch] * fb_gain) * un_pump;
            let dry = z.mul_add(f64::from(insert[ch] - x), f64::from(x)) as f32;
            out[ch] = wet_out + dry * dry_gain;
        }
        (out[0], out[1])
    }
}
