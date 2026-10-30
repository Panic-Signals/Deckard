//! Flanger: Maschine's Perform FX Flanger, as docs/fx-perform-flanger.md describes it. A
//! stereo comb (`Comb`): the input, gated by the strip, plus the delayed signal times
//! the feedback gain and the Polarity's sign, through a tanh saturator, a one-pole
//! high-pass, in Phaser tone sixteen one-pole all-passes, into a delay line read with
//! an all-pass fractional tap. The strip sets the delay (a pitch, 10–135 semitones over
//! MIDI note 0) and the all-passes' frequency; Decay the time the feedback takes to fall
//! 60 dB; Stereo how much longer the right side's delay is. Out: the input plus the
//! comb's output, levelled by Saturation and Wet Level. Released, only the tail is added.

use super::{Ctx, Mode, Params};
use std::f64::consts::PI;

/// `10^x` as Apple's libm computes it (Maschine's `__exp10`).
fn exp10(x: f64) -> f64 {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn __exp10(x: f64) -> f64;
        }
        unsafe { __exp10(x) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        10f64.powf(x)
    }
}

/// 1/12, as Maschine's code has it.
const TWELFTH: f64 = f64::from_bits(0x3fb5_5555_5555_5555);
/// MIDI note 0's frequency, as a power of two (8.1758 Hz).
const NOTE0: f64 = f64::from_bits(0x4008_4039_7fa8_81bf);

/// tan(w) ≈ w + w³/3 + 2w⁵/15, the one-pole's prewarped gain, and 1/(1 + g).
fn warp(w: f64) -> (f64, f64) {
    let w2 = w * w;
    let w3 = w * w2;
    let g = w3.mul_add(f64::from_bits(0x3fd5_5555_5555_554f), w);
    let g = (w2 * w3).mul_add(f64::from_bits(0x3fc1_1111_1111_1111), g);
    (g, 1.0 / (g + 1.0))
}

/// A one-pole in trapezoidal form, state `s`: (high-pass, low-pass) of `x`.
fn one_pole(s: &mut f64, x: f64, g: f64, gg: f64) -> (f64, f64) {
    let v = x + gg * (-x).mul_add(g, *s * -2.0);
    let lp = (*s * 2.0) + g * v;
    *s = v.mul_add(g, *s);
    (v, lp)
}

/// A value gliding linearly to its target in a set number of samples (Maschine's
/// parameter smoothing).
#[derive(Debug, Clone, Copy, Default)]
struct Glide {
    v: f32,
    target: f32,
    step: f32,
    steps: f32,
    count: i32,
    left: i32,
    moving: bool,
}

impl Glide {
    /// Glides take `steps` samples.
    fn time(&mut self, steps: f32) {
        self.steps = steps;
        self.count = steps.round() as i32;
        self.left = self.count;
    }

    fn to(&mut self, t: f32) {
        self.target = t;
        if self.v != t {
            self.step = (t - self.v) / self.steps;
            self.left = self.count;
            self.moving = true;
        }
    }

    /// At the target now.
    fn land(&mut self) {
        if self.moving {
            self.v = self.target;
            self.step = 0.0;
            self.left = 0;
            self.moving = false;
        }
    }

    fn next(&mut self) -> f32 {
        let done = (self.target - self.v).abs() <= 0.0 || {
            let n = self.left;
            self.left -= 1;
            n < 2
        };
        if done {
            self.land();
        } else {
            self.v += self.step;
        }
        self.v
    }
}

/// [`Glide`] in double precision (the Polarity's sign).
#[derive(Debug, Clone, Copy, Default)]
struct GlideD {
    v: f64,
    target: f64,
    step: f64,
    steps: f64,
    count: i32,
    left: i32,
    moving: bool,
}

impl GlideD {
    fn to(&mut self, t: f64) {
        self.target = t;
        if self.v != t {
            self.step = (t - self.v) / self.steps;
            self.left = self.count;
            self.moving = true;
        }
    }

    fn land(&mut self) {
        if self.moving {
            self.v = self.target;
            self.step = 0.0;
            self.left = 0;
            self.moving = false;
        }
    }

    fn next(&mut self) -> f64 {
        let done = (self.target - self.v).abs() <= 0.0 || {
            let n = self.left;
            self.left -= 1;
            n < 2
        };
        if done {
            self.land();
        } else {
            self.v += self.step;
        }
        self.v
    }
}

/// One one-pole stage: its state per side, its gains.
#[derive(Debug, Clone, Copy, Default)]
struct Stage {
    s: [f64; 2],
    g: f64,
    gg: f64,
}

/// The comb (Maschine's `CombFilter<double>`).
struct Comb {
    /// The delay lines (a power of two long) and where each writes next.
    buf: [Vec<f64>; 2],
    mask: usize,
    w: [usize; 2],
    /// The tap: whole samples back, the fraction's all-pass coefficient, its state, and
    /// the last sample's whole delay.
    d: [i32; 2],
    coef: [f64; 2],
    prev: [f64; 2],
    last_d: [i32; 2],
    /// What feeds back next, and the feedback gain per side.
    fb: [f64; 2],
    fb_gain: [f64; 2],
    sign: GlideD,
    drive: f64,
    undrive: f64,
    /// The high-pass (60 or 90 Hz) and the all-passes (Phaser: sixteen).
    hp: Stage,
    stages: [Stage; 16],
    n: usize,
    inv_sr: f32,
    /// The all-passes: the first at `freq · lowest`, each next `spacing` higher.
    freq: f32,
    spread: f32,
    spacing: f32,
    lowest: f32,
}

impl Comb {
    fn new(sr: f32) -> Comb {
        let inv_sr = 1.0 / sr;
        let len = (4.0 * sr) as usize;
        let size = len.next_power_of_two();
        let mut c = Comb {
            buf: [vec![0.0; size], vec![0.0; size]],
            mask: size - 1,
            w: [0; 2],
            d: [1; 2],
            coef: [0.5; 2],
            prev: [0.0; 2],
            last_d: [1; 2],
            fb: [0.0; 2],
            fb_gain: [0.0; 2],
            sign: GlideD::default(),
            drive: 1.0,
            undrive: 1.0,
            hp: Stage::default(),
            stages: [Stage::default(); 16],
            n: 0,
            inv_sr,
            freq: 1000.0,
            spread: 48.0,
            spacing: 256.0,
            lowest: 0.0625,
        };
        let steps = f64::from(sr) * f64::from_bits(0x3f50_624d_d2f1_a9fc) * 20.0;
        c.sign.steps = steps;
        c.sign.count = steps.round() as i32;
        c.sign.left = c.sign.count;
        c.high_pass(10.0);
        c
    }

    fn high_pass(&mut self, hz: f32) {
        let (g, gg) = warp(f64::from(hz) * PI * f64::from(self.inv_sr));
        (self.hp.g, self.hp.gg) = (g, gg);
    }

    /// The all-passes' gains, from `freq`.
    fn tune(&mut self) {
        let mut f = self.freq * self.lowest;
        for st in &mut self.stages[..self.n] {
            (st.g, st.gg) = warp(f64::from(f) * PI * f64::from(self.inv_sr));
            f *= self.spacing;
        }
    }

    /// Flanger (0) or Phaser (16) all-passes; a change clears the comb.
    fn stages(&mut self, n: usize) {
        let n = n.min(16);
        if n == self.n {
            return;
        }
        self.n = n;
        let share = if n == 0 { 1.0 } else { 1.0 / n as f32 };
        self.spacing = (f64::from(share * (self.spread + self.spread)) * TWELFTH).exp2() as f32;
        self.lowest = (f64::from(-self.spread) * TWELFTH).exp2() as f32;
        for st in &mut self.stages {
            st.s = [0.0; 2];
        }
        self.prev = [0.0; 2];
        self.fb = [0.0; 2];
        for b in &mut self.buf {
            b.fill(0.0);
        }
        self.w = [0; 2];
        self.hp.s = [0.0; 2];
        self.sign.land();
        self.tune();
    }

    /// One frame: `x` in, the delayed signal out.
    fn frame(&mut self, x: [f64; 2]) -> [f32; 2] {
        let p = self.sign.next();
        let mut v = [0.0; 2];
        for ch in 0..2 {
            let mut y = self.fb[ch] * p + x[ch];
            y = (self.drive * y).tanh() * self.undrive;
            let (hp, _) = one_pole(&mut self.hp.s[ch], y, self.hp.g, self.hp.gg);
            y = hp;
            for st in &mut self.stages[..self.n] {
                let (hp, lp) = one_pole(&mut st.s[ch], y, st.g, st.gg);
                y = lp - hp;
            }
            v[ch] = y;
        }
        let mut out = [0.0f32; 2];
        for ch in 0..2 {
            let b = &mut self.buf[ch];
            b[self.w[ch]] = v[ch];
            self.w[ch] = (self.w[ch] + 1) & self.mask;
            let at = |d: i32| self.w[ch].wrapping_sub(d as isize as usize) & self.mask;
            let s = b[at(self.d[ch])] as f32;
            let t = (self.coef[ch] * (f64::from(s) - self.prev[ch])) as f32;
            let y = if self.d[ch] < self.last_d[ch] { b[at(self.last_d[ch])] } else { self.prev[ch] + f64::from(t) };
            self.prev[ch] = f64::from(s + t);
            self.last_d[ch] = self.d[ch];
            out[ch] = y as f32;
        }
        self.fb = [self.fb_gain[0] * f64::from(out[0]), self.fb_gain[1] * f64::from(out[1])];
        out
    }

    /// The tap `secs` back on side `ch`: whole samples and the fraction's coefficient.
    fn delay(&mut self, ch: usize, secs: f32, sr: f32) {
        let at = sr * secs;
        self.d[ch] = at as i32;
        let frac = f64::from(at - (at as i32) as f32);
        self.coef[ch] = (1.0 - frac) / (frac + 1.0);
    }
}

pub struct Flanger {
    sr: f32,
    /// The parameters as last set, by core index (0: strip, 1: engage, 2: Decay,
    /// 3: Polarity, 4: Tone, 5: Stereo, 6: Saturation, 7: Wet Level).
    p: [f32; 8],
    /// Set before the first frame: values land at once.
    fresh: bool,
    /// The strip's pitch (semitones over MIDI note 0) and its delay.
    pitch: f32,
    delay: f32,
    phaser: bool,
    stereo: Glide,
    ratio: Glide,
    engage: Glide,
    sat: Glide,
    wet: Glide,
    target: Glide,
    freq: Glide,
    decay: Glide,
    fade: Glide,
    /// The delay's 20 Hz smoothing: state, gains.
    lp: f64,
    lp_g: f64,
    lp_gg: f64,
    /// What the comb was last set for: delay, stereo, decay.
    was: (f32, f32, f32),
    /// Samples into the control period (Decay moves every fourth).
    tick: u32,
    /// Recompute everything (after a reset); the tone changed; the delay changed.
    force: bool,
    retone: bool,
    redelay: bool,
    comb: Comb,
}

impl Flanger {
    pub fn new(sr: f32) -> Flanger {
        let ms = sr * 0.001;
        let mut f = Flanger {
            sr,
            p: [0.0; 8],
            fresh: true,
            pitch: 1.0,
            delay: 0.001,
            phaser: false,
            stereo: Glide::default(),
            ratio: Glide::default(),
            engage: Glide::default(),
            sat: Glide::default(),
            wet: Glide::default(),
            target: Glide::default(),
            freq: Glide::default(),
            decay: Glide::default(),
            fade: Glide::default(),
            lp: 0.0,
            lp_g: 0.0,
            lp_gg: 0.0,
            was: (1.0, -1.0, 0.0),
            tick: 0,
            force: true,
            retone: true,
            redelay: true,
            comb: Comb::new(sr),
        };
        for g in [&mut f.stereo, &mut f.ratio, &mut f.wet, &mut f.sat] {
            g.time(ms * 40.0);
        }
        f.decay.time(sr / 4.0 * 0.001 * 50.0);
        f.engage.time(ms * 10.0);
        f.target.time(ms * 15.0);
        f.freq.time(ms * 15.0);
        f.fade.time(sr * 0.001 * 0.1);
        (f.lp_g, f.lp_gg) = warp(f64::from(1.0 / sr) * f64::from_bits(0x404f_6a7a_2955_385e));
        f.reset();
        f
    }

    /// Back to silence (Maschine's reset): the comb cleared, the input fading in.
    fn reset(&mut self) {
        self.force = true;
        self.redelay = true;
        let c = &mut self.comb;
        for st in &mut c.stages {
            st.s = [0.0; 2];
        }
        c.hp.s = [0.0; 2];
        c.fb = [0.0; 2];
        for b in &mut c.buf {
            b.fill(0.0);
        }
        c.w = [0; 2];
        c.prev = [0.0; 2];
        c.sign.land();
        self.lp = f64::from(self.delay) * 0.5;
        self.was.2 = 0.0;
        self.was.0 = 1.0;
        self.tick = 0;
        self.fade_in();
    }

    fn fade_in(&mut self) {
        self.fade.v = 0.0;
        self.fade.target = 1.0;
        self.fade.step = 1.0 / self.fade.steps;
        self.fade.left = self.fade.count;
        self.fade.moving = true;
    }

    /// Before the first frame, every glide lands at once.
    fn land(&mut self) {
        if self.fresh {
            for g in [
                &mut self.target,
                &mut self.freq,
                &mut self.decay,
                &mut self.stereo,
                &mut self.ratio,
                &mut self.wet,
                &mut self.engage,
                &mut self.sat,
            ] {
                g.land();
            }
        }
    }

    /// Core parameter `i` (see [`Flanger::p`]) set to `v`.
    fn param(&mut self, i: usize, v: f32) {
        self.p[i] = v;
        match i {
            0 => self.strip(),
            1 => self.engage.to(if v > 0.5 { 1.0 } else { 0.0 }),
            2 => self.set_decay(),
            3 => {
                self.comb.sign.to(if v > 0.5 { 1.0 } else { -1.0 });
                self.comb.high_pass(if v > 0.5 { 90.0 } else { 60.0 });
            }
            4 => {
                if self.phaser != (v > 0.5) {
                    self.retone = true;
                    self.phaser = v > 0.5;
                }
            }
            5 => self.stereo.to((v.abs() + 0.0) * v),
            6 => self.sat.to((f64::from(v.clamp(0.0, 1.0)).mul_add(24.0, -12.0)) as f32),
            _ => self.wet.to(v + v),
        }
        self.land();
    }

    /// The strip: the delay's pitch, the all-passes' frequency, the sides' ratio; Decay
    /// and Wet Level follow.
    fn strip(&mut self) {
        let x = self.p[0];
        self.pitch = f64::from(x.clamp(0.0, 1.0)).mul_add(125.0, 10.0) as f32;
        let bent = (x.abs().mul_add(0.5, 0.5) * x).clamp(0.0, 1.0);
        let tone = f64::from(bent).mul_add(95.0, 10.0) as f32;
        let hz = f64::from(self.pitch).mul_add(TWELFTH, NOTE0).exp2() as f32;
        let freq = f64::from(tone).mul_add(TWELFTH, NOTE0).exp2() as f32;
        self.delay = 1.0 / hz;
        let t = ((self.pitch - 10.0) / 115.0).clamp(0.0, 1.0).max(0.0);
        self.ratio.to(f64::from(t).mul_add(f64::from(0.199_999_93f32), f64::from(1.1f32)) as f32);
        self.target.to(self.delay);
        self.freq.to(freq);
        self.set_decay();
        self.wet.to(self.p[7] + self.p[7]);
    }

    /// Decay: how long the feedback takes to fall 60 dB (seconds), by the knob and the
    /// pitch.
    fn set_decay(&mut self) {
        let x = self.p[2];
        let lo = ((self.pitch - 20.0) / 110.0).clamp(0.0, 1.0);
        let hi = ((self.pitch - 10.0) / 120.0).clamp(0.0, 1.0);
        let top = f64::from(hi).mul_add(-6.0, 3.0) as f32;
        let floor = f64::from(lo).mul_add(-33.0, 53.0) as f32;
        let db = f64::from(x.clamp(0.0, 1.0)).mul_add(f64::from(75.0 - floor), f64::from(floor)) as f32;
        let ms = exp10(f64::from(top + db) * 0.05);
        let secs = (ms * f64::from_bits(0x3f50_624d_d2f1_a9fc)) as f32;
        self.decay.to((x / 0.2).clamp(0.0, 1.0) * secs);
    }
}

impl Mode for Flanger {
    fn set(&mut self, p: &Params) {
        let fields = [
            "touchstrip_pos",
            "touchstrip_on_off",
            "flanger_decay",
            "flanger_polarity",
            "flanger_tone",
            "flanger_stereo",
            "flanger_saturation",
            "flanger_wet_level",
        ];
        // Maschine sends them strip first, then engage, Polarity, Decay, Stereo, Tone,
        // Saturation, Wet Level.
        for i in [0, 1, 3, 2, 5, 4, 6, 7] {
            let v = p.f(fields[i]);
            let v = if i == 1 { if v >= 0.5 { 1.0 } else { 0.0 } } else { v };
            if self.fresh || v != self.p[i] {
                self.param(i, v);
            }
        }
    }

    fn frame(&mut self, l: f32, r: f32, _: &Ctx) -> (f32, f32) {
        if self.fresh {
            self.fresh = false;
        }
        if self.retone {
            self.comb.stages(if self.phaser { 16 } else { 0 });
            self.fade_in();
            self.retone = false;
        }
        let stereo = self.stereo.next();
        let ratio = self.ratio.next();
        let engage = self.engage.next();
        let sat = self.sat.next();
        let wet = self.wet.next();
        let target = self.target.next();
        let freq = self.freq.next();
        let decay = if self.tick == 0 { self.decay.next() } else { self.was.2 };
        // The delay, smoothed at 20 Hz.
        let (v, out) = {
            let x = f64::from(target);
            let v = x + self.lp_gg * (-x).mul_add(self.lp_g, self.lp * -2.0);
            (v, (self.lp * 2.0) + self.lp_g * v)
        };
        self.lp = v.mul_add(self.lp_g, self.lp);
        let delay = out as f32;
        self.tick = (self.tick + 1) & 3;
        if self.was.0 != delay || self.was.1 != stereo || self.force {
            let right = delay * (f64::from(stereo).mul_add(f64::from(ratio + -1.0), 1.0) as f32);
            self.comb.freq = freq;
            self.comb.tune();
            self.comb.delay(0, delay, self.sr);
            self.comb.delay(1, right, self.sr);
            (self.was.0, self.was.1) = (delay, stereo);
            self.redelay = true;
        }
        if decay != self.was.2 || self.force || (self.redelay && self.tick == 0) {
            let g = exp10(f64::from(delay) * -3.0 / f64::from(decay + 1e-10)) as f32;
            self.comb.fb_gain = [f64::from(g); 2];
            self.was.2 = decay;
            self.redelay = false;
        }
        let drive = exp10(f64::from(sat) * 0.05) as f32;
        self.comb.drive = f64::from(drive);
        self.comb.undrive = 1.0 / f64::from(drive);
        let gate = self.fade.next() * engage;
        let y = self.comb.frame([f64::from(gate * l), f64::from(gate * r)]);
        let level = f64::from((exp10(f64::from(sat * 0.6) * 0.05) as f32) * 0.4);
        self.force = false;
        (wet.mul_add((f64::from(y[0]) * level) as f32, l), wet.mul_add((f64::from(y[1]) * level) as f32, r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Maschine's Perform FX defaults with Flanger chosen, `extra` on top.
    fn params(extra: &[(&str, f64)]) -> Vec<(String, f64)> {
        let mut p: Vec<(String, f64)> = deckard_model::PERFORM_FX.1.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        for (k, v) in [("mode", 1.0), ("touchstrip_on_off", 1.0)].iter().chain(extra) {
            match p.iter_mut().find(|(n, _)| n == k) {
                Some(x) => x.1 = *v,
                None => p.push((k.to_string(), *v)),
            }
        }
        p
    }

    fn ctx(sr: f32) -> Ctx {
        Ctx { sr, pos: 0.5, engaged: true, quarter: sr / 2.0, song: None }
    }

    fn run(f: &mut Flanger, sr: f32, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let c = ctx(sr);
        l.iter().zip(r).map(|(a, b)| f.frame(*a, *b, &c)).unzip()
    }

    fn read(path: &str) -> Option<(f32, Vec<f32>, Vec<f32>)> {
        let mut rd = hound::WavReader::open(path).ok()?;
        let sr = rd.spec().sample_rate as f32;
        let x: Vec<f32> = match rd.spec().sample_format {
            hound::SampleFormat::Float => rd.samples::<f32>().map(Result::unwrap).collect(),
            hound::SampleFormat::Int => {
                let s = (1u64 << (rd.spec().bits_per_sample - 1)) as f32;
                rd.samples::<i32>().map(|v| v.unwrap() as f32 / s).collect()
            }
        };
        Some((sr, x.iter().step_by(2).copied().collect(), x.iter().skip(1).step_by(2).copied().collect()))
    }

    /// Residual of `a − b` in dB (RMS, both sides).
    fn residual(a: &[f32], b: &[f32]) -> f64 {
        let n = a.len().min(b.len()).max(1);
        let e: f64 = a.iter().zip(b).map(|(x, y)| ((x - y) as f64).powi(2)).sum::<f64>() / n as f64;
        10.0 * e.max(1e-24).log10()
    }

    /// The reference renders (out/perform-flanger/render.sh: Maschine 3 through
    /// maschine-ref, 1 s of silence first): each case nulls against Maschine.
    /// Skipped where the renders aren't there.
    #[test]
    fn nulls_against_maschine() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../out/perform-flanger");
        let cases: &[(&str, &[(&str, f64)])] = &[
            ("def", &[]),
            ("phaser", &[("flanger_tone", 1.0)]),
            ("pos0", &[("touchstrip_pos", 0.0)]),
            ("pos1", &[("touchstrip_pos", 1.0)]),
            ("pos03", &[("touchstrip_pos", 0.3)]),
            ("posit", &[("flanger_polarity", 1.0)]),
            ("dec0", &[("flanger_decay", 0.0)]),
            ("dec1", &[("flanger_decay", 1.0), ("touchstrip_pos", 0.8)]),
            ("dec01", &[("flanger_decay", 0.1)]),
            ("ster0", &[("flanger_stereo", 0.0), ("flanger_wet_level", 1.0)]),
            ("sat0", &[("flanger_saturation", 0.0)]),
            ("sat1", &[("flanger_saturation", 1.0), ("flanger_wet_level", 1.0)]),
            ("off", &[("touchstrip_on_off", 0.0)]),
            ("r48", &[("touchstrip_pos", 0.65), ("flanger_tone", 1.0), ("flanger_polarity", 1.0)]),
            ("imp", &[("touchstrip_pos", 0.2), ("flanger_decay", 0.9)]),
        ];
        let mut worst: Vec<(String, f64)> = Vec::new();
        for (name, extra) in cases {
            let (Some((sr, il, ir)), Some((_, ol, or))) = (read(&format!("{dir}/{name}-in.wav")), read(&format!("{dir}/{name}.wav")))
            else {
                continue;
            };
            let mut f = Flanger::new(sr);
            f.set(&Params(&params(extra)));
            let pre = vec![0.0; sr as usize];
            run(&mut f, sr, &pre, &pre);
            let (l, r) = run(&mut f, sr, &il, &ir);
            let db = residual(&l, &ol).max(residual(&r, &or));
            eprintln!("{name}: {db:.1} dB");
            worst.push((name.to_string(), db));
        }
        let bad: Vec<_> = worst.iter().filter(|(_, db)| *db > -200.0).collect();
        assert!(bad.is_empty(), "{bad:?}");
    }

    /// Released, the input passes untouched once the echoes have died away.
    #[test]
    fn released_it_passes_the_input() {
        let sr = 44_100.0;
        let mut f = Flanger::new(sr);
        f.set(&Params(&params(&[("touchstrip_on_off", 0.0)])));
        let x: Vec<f32> = (0..20_000).map(|i| ((i * 7919 % 1000) as f32 / 1000.0 - 0.5) * 0.5).collect();
        let (l, r) = run(&mut f, sr, &x, &x);
        assert_eq!(l, x);
        assert_eq!(r, x);
    }

    /// Engaged, the comb rings: an impulse comes back after the strip's delay.
    #[test]
    fn engaged_it_rings() {
        let sr = 44_100.0;
        let mut f = Flanger::new(sr);
        f.set(&Params(&params(&[("flanger_decay", 0.9)])));
        let mut x = vec![0.0; 20_000];
        x[1000] = 1.0;
        let (l, _) = run(&mut f, sr, &x, &x);
        let tail: f32 = l[1100..].iter().map(|v| v * v).sum();
        assert!(tail > 1e-3, "{tail}");
    }
}
