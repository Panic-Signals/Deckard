//! Ring: Maschine's Perform FX ring modulator (docs/fx-perform-ring.md). Four sine
//! oscillators, pitched by the strip and fanned apart by Spread, each ring-modulate the
//! input; each product runs through a resonant band pass tuned between the base pitch
//! and its oscillator, and the bands are panned and summed. Dry/Wet and an engage
//! envelope (5 ms up, 100 ms down) scale the sum into Maschine's Plate reverb, fully
//! wet; Rev Mix crossfades between the ring sound and the reverb. Glide slides the
//! pitches. Released, the reverb rings on and the input passes.
//!
//! The reverb is `NI::DSP::PlateReverb`, the Plate engine of Maschine's Reverb module;
//! DECKARD's port of it (`fx::plate_reverb::plate`) is compiled here as a private copy of
//! the same file, since that module keeps it to itself.

use super::{Ctx, Mode, Params};
use crate::fx::plate_reverb::c;

use crate::fx::plate_reverb::plate::Plate;

/// `__exp10` from the system math library, which Maschine calls (10^x, f64).
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

fn d(bits: u64) -> f64 {
    f64::from_bits(bits)
}

/// log2 of MIDI note 0's frequency (8.18 Hz): note `n` is `2^(n/12 + this)` Hz.
const NOTE0: u64 = 0x4008_4039_7fa8_81bf;
const TWELFTH: u64 = 0x3fb5_5555_5555_5555;
/// The tan series `x + x³/3 + 2x⁵/15`.
const TAN3: u64 = 0x3fd5_5555_5555_554f;
const TAN5: u64 = 0x3fc1_1111_1111_1111;
const PI: f64 = std::f64::consts::PI;
/// The processor's control-rate period (samples).
const BLOCK: u32 = 128;

/// `tan(x)` as Maschine approximates it.
fn tan_series(x: f64) -> f64 {
    let x2 = x * x;
    let x3 = x * x2;
    let x5 = x2 * x3;
    x5.mul_add(d(TAN5), x3.mul_add(d(TAN3), x))
}

/// A note number's frequency (Hz).
fn note_hz(n: f32) -> f32 {
    (n as f64).mul_add(d(TWELFTH), d(NOTE0)).exp2() as f32
}

fn clamp01(v: f32) -> f32 {
    let a = if v > 1.0 { 1.0 } else { v };
    if v < 0.0 { 0.0 } else { a }
}

/// A smoothed parameter: a straight line to its target over a fixed number of samples.
#[derive(Clone, Copy, Default)]
struct Glide {
    cur: f32,
    tgt: f32,
    inc: f32,
    steps: f32,
    countdown: i32,
    reload: i32,
    ramping: bool,
}

impl Glide {
    fn set(&mut self, v: f32) {
        self.tgt = v;
        if self.cur != self.tgt {
            self.inc = (v - self.cur) / self.steps;
            self.countdown = self.reload;
            self.ramping = true;
        }
    }

    fn snap(&mut self) {
        if self.ramping {
            self.cur = self.tgt;
            self.inc = 0.0;
            self.countdown = 0;
            self.ramping = false;
        }
    }

    fn tick(&mut self) {
        let done = (self.tgt - self.cur).abs() <= 0.0 || {
            let n = self.countdown;
            self.countdown = n.wrapping_sub(1);
            n < 2
        };
        if done {
            self.snap();
        } else {
            self.cur += self.inc;
        }
    }

    /// `ms` long at `sr`.
    fn length(&mut self, sr: f32, ms: f32) {
        self.steps = sr * c(0x3a83_126f) * ms;
        self.reload = self.steps.round() as i32;
        self.countdown = self.reload;
    }
}

// The smoothed parameters, in Maschine's order (§2).
/// The strip's position as a frequency (`2^(pos/12)` × 8.18 Hz): shapes the panning.
const G_RAW: usize = 0;
/// The base pitch's frequency, then the four oscillators'.
const G_BASE: usize = 1;
const G_OSC: usize = 2;
const G_ENGAGE: usize = 14;
const G_REV_MIX: usize = 15;
const G_GLIDE: usize = 16;
/// 1 in Mix mode, 0 in Wet only: the dry level while released.
const G_MIX: usize = 17;
const G_DRY: usize = 18;
const G_WET: usize = 19;
const G_SPREAD: usize = 20;
const GLIDES: usize = 21;

/// A band pass (trapezoidal state-variable filter), both channels.
#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: [f64; 2],
    ic2: [f64; 2],
}

pub struct Ring {
    sr: f32,
    /// `setParameter` values as last given, by Maschine's index: 0 position, 1 engage,
    /// 2 Spread, 3 Mix Mode, 4 Dry/Wet, 5 Rev Mix, 6 Rev Time, 7 Glide.
    p: [f32; 8],
    seen: bool,
    /// Parameters land at once instead of gliding (before the first resume).
    immediate: bool,
    spread: f32,
    g: [Glide; GLIDES],
    /// The pitch filters' rate as last used (−1: work them out again).
    glide_at: f32,
    /// One-pole filters on the frequencies: osc 1–4, base, raw.
    lp_s: [f64; 6],
    lp_g: f64,
    lp_a: f64,
    lp_t: f32,
    /// The oscillators: rotating phasors.
    osc_x: [f64; 4],
    osc_y: [f64; 4],
    svf: [Svf; 4],
    svf_t: f64,
    env: f64,
    attack: f64,
    release: f64,
    plate: Box<Plate>,
    /// Samples left in the current control-rate block.
    until: u32,
}

impl Ring {
    pub fn new(sr: f32) -> Ring {
        // The constructor and onInit (§3.1): the reverb fully wet, at 1.5×.
        let mut plate = Plate::new();
        plate.set_parameter(0, 0.0);
        plate.set_parameter(7, 1.0);
        plate.set_parameter(8, 1.5);
        plate.set_parameter(9, 0.5);
        plate.set_parameter(1, 0.5);
        plate.set_parameter(10, 0.2);
        let mut r = Ring {
            sr,
            p: [0.0; 8],
            seen: false,
            immediate: true,
            spread: 0.0,
            g: [Glide::default(); GLIDES],
            glide_at: -1.0,
            lp_s: [0.0; 6],
            lp_g: 0.0,
            lp_a: 0.0,
            lp_t: 1.0,
            osc_x: [0.0; 4],
            osc_y: [0.0; 4],
            svf: [Svf::default(); 4],
            svf_t: 0.0,
            env: 0.0,
            attack: 0.0,
            release: 0.0,
            plate,
            until: 0,
        };
        r.set_sample_rate(sr);
        r
    }

    /// `onSetSampleRate(sr, 128)` (§3.2).
    fn set_sample_rate(&mut self, sr: f32) {
        if sr < 44_100.0 {
            return;
        }
        self.sr = sr;
        for (i, g) in self.g.iter_mut().enumerate() {
            g.length(sr, if i <= G_OSC + 3 { 5.0 } else { 20.0 });
        }
        self.svf_t = 1.0 / sr as f64;
        let t = 1.0 / sr;
        if t != self.lp_t {
            self.lp_t = t;
            self.lp_s = [0.0; 6];
        }
        let ln = d(0xc002_6bb1_bbb5_317f);
        let sr64 = sr as f64;
        self.attack = 1.0 - (ln / (sr64 * d(0x3f74_7ae1_4000_0000))).exp();
        self.release = 1.0 - (ln / (sr64 * d(0x3fb9_9999_a000_0000))).exp();
        self.set_parameter(0, self.p[0]);
        self.plate.set_sample_rate(sr, BLOCK as i32);
    }

    /// `setTouchstripPosInternal(pos)` (§2.1): the five pitches and the raw one.
    fn pitches(&mut self, pos: f32) {
        let x = clamp01(pos) as f64;
        let base = x.mul_add(70.0, 30.0) as f32;
        let n2 = x.mul_add(d(0x4044_6666_6000_0000), d(0x4031_4ccc_c000_0000)) as f32;
        let n3 = x.mul_add(d(0x4049_b333_2000_0000), d(0x403c_cccc_c000_0000)) as f32;
        let lin = x.mul_add(d(0xc025_3333_0000_0000), 100.0) as f32;
        let n4 = (((lin as f64).log2() + d(0xc008_4039_7fa8_81bf)) * 12.0) as f32;
        let n5 = x.mul_add(d(0x404a_3333_2000_0000), d(0x404e_5999_a000_0000)) as f32;
        let s = clamp01(self.spread) as f64;
        let b = base as f64;
        let fan = |n: f32| s.mul_add((n - base) as f64, b) as f32;
        let notes = [fan(n2), fan(n3), fan(n4), fan(n5)];
        self.g[G_BASE].set(note_hz(base));
        self.g[G_RAW].set(note_hz(pos));
        for (k, n) in notes.into_iter().enumerate() {
            self.g[G_OSC + k].set(note_hz(n));
        }
    }

    /// `setParameter(index, value)` (§1).
    fn set_parameter(&mut self, i: usize, v: f32) {
        self.p[i] = v;
        match i {
            0 => self.pitches(v),
            1 => self.g[G_ENGAGE].set(if v > 0.5 { 1.0 } else { 0.0 }),
            2 => {
                self.spread = v;
                self.g[G_SPREAD].set(clamp01(v / c(0x3e19_999a)));
                self.pitches(self.p[0]);
            }
            3 => {
                self.g[G_MIX].set(if v > 0.5 { 0.0 } else { 1.0 });
                self.set_parameter(4, self.p[4]);
            }
            4 => {
                let x = v + -0.5;
                let x = x + x;
                let dry = if x > 1.0 { 0.0 } else { 1.0 - x };
                let dry = if x < 0.0 { 1.0 } else { dry };
                let wet = clamp01(v + v);
                let wet_only = if self.p[3] > 0.5 { 1.0 } else { 0.0 };
                self.g[G_DRY].set(dry * (1.0 - wet_only));
                self.g[G_WET].set(wet);
            }
            5 => self.g[G_REV_MIX].set(v),
            6 => {
                let decay = (v.abs().mul_add(0.75, 0.25) * v) * c(0x3f66_6666);
                self.plate.set_parameter(1, decay);
            }
            7 => {
                let rate = if v <= c(0x3c23_d70a) {
                    1.0 / c(0x3b83_126f)
                } else {
                    let db = (clamp01(v) as f64).mul_add(40.0, 30.0) as f32;
                    let ms = exp10(db as f64 * d(0x3fa9_9999_9999_999a)) as f32;
                    1.0 / (ms * c(0x3a83_126f))
                };
                self.g[G_GLIDE].set(rate);
            }
            _ => {}
        }
        if self.immediate {
            for g in &mut self.g {
                g.snap();
            }
        }
    }

    /// `onReset()` (§3.3): states cleared; the pitch filters run in on their targets.
    fn reset(&mut self) {
        for g in &mut self.g {
            g.snap();
        }
        self.svf = [Svf::default(); 4];
        self.osc_x = [1.0; 4];
        self.osc_y = [0.0; 4];
        self.plate.reset();
        let g = tan_series(self.lp_t as f64 * d(0x40ee_adfb_4c5d_390c));
        let a = 1.0 / (g + 1.0);
        let ins = [G_OSC, G_OSC + 1, G_OSC + 2, G_OSC + 3, G_BASE, G_RAW].map(|i| self.g[i].tgt as f64);
        let mut s = [0.0f64; 6];
        for _ in 0..20 {
            for k in 0..6 {
                let t = ins[k].mul_add(-g, s[k] * -2.0);
                let u = t * a + ins[k];
                s[k] = u.mul_add(g, s[k]);
            }
        }
        self.lp_s = s;
        self.glide_at = -1.0;
    }

    /// One sample of `processAudio` (§4).
    fn sample(&mut self, l: f32, r: f32) -> (f32, f32) {
        for g in &mut self.g {
            g.tick();
        }
        // 1. The pitch filters (Glide).
        let rate = self.g[G_GLIDE].cur;
        if rate != self.glide_at {
            self.lp_g = tan_series((rate as f64 * PI) * self.lp_t as f64);
            self.lp_a = 1.0 / (self.lp_g + 1.0);
            self.glide_at = rate;
        }
        let ins = [G_OSC, G_OSC + 1, G_OSC + 2, G_OSC + 3, G_BASE, G_RAW].map(|i| self.g[i].cur as f64);
        let (g, a) = (self.lp_g, self.lp_a);
        let mut f = [0.0f64; 6];
        for k in 0..6 {
            let s = self.lp_s[k];
            let t = ins[k].mul_add(-g, s * -2.0);
            let u = a * t + ins[k];
            f[k] = 2.0f64.mul_add(s, g * u);
            self.lp_s[k] = u.mul_add(g, s);
        }
        let base = f[4] as f32;
        let raw = f[5] as f32;
        let hz = [f[0] as f32, f[1] as f32, f[2] as f32, f[3] as f32];
        // 2. The band passes, tuned between the base pitch and each oscillator.
        let spread = self.spread as f64;
        let k_res = d(0x3ff3_3333_3000_0000);
        let coef = hz.map(|h| {
            let fc = spread.mul_add((h - base) as f64, base as f64) as f32 as f64;
            let g = tan_series((fc * PI) * self.svf_t);
            let a1 = 1.0 / (k_res + g).mul_add(g, 1.0);
            let a2 = g * a1;
            (a1, a2, g * a2)
        });
        // 3. The oscillators.
        let mut osc = [0f32; 4];
        for k in 0..4 {
            let w = (hz[k] as f64 * std::f64::consts::TAU) / self.sr as f64;
            let (sn, cs) = w.sin_cos();
            let (x, y) = (self.osc_x[k], self.osc_y[k]);
            self.osc_x[k] = cs.mul_add(x, -(sn * y)) + 0.0;
            self.osc_y[k] = sn.mul_add(x, cs * y);
            osc[k] = self.osc_y[k] as f32;
        }
        // 4. Ring modulation into the band passes.
        let inp = [l as f64, r as f64];
        let mut band = [[0.0f64; 2]; 4];
        for k in 0..4 {
            let (a1, a2, a3) = coef[k];
            let o = osc[k] as f64;
            for ch in 0..2 {
                let st = &mut self.svf[k];
                let (ic1, ic2) = (st.ic1[ch], st.ic2[ch]);
                let v3 = inp[ch] * o - ic2;
                let v1 = ic1.mul_add(a1, a2 * v3);
                let v2 = ic2 + a2.mul_add(ic1, a3 * v3);
                st.ic1[ch] = v1.mul_add(2.0, -ic1);
                st.ic2[ch] = v2.mul_add(2.0, -ic2);
                band[k][ch] = v1;
            }
        }
        // 5. Panning: band 1 by Spread, bands 2–4 by triangles of the strip.
        let tri = |a: f32| {
            let w = ((a + 10.5) as i32 - 10) as f32;
            ((a - w).abs() + -0.25) * 4.0
        };
        let p2 = tri(raw.mul_add(c(0x3fa6_6666), c(0x3dcc_cccd))) * c(0x3f19_999a);
        let p3 = tri(raw.mul_add(c(0x3fa6_6666), c(0xbe99_999a))) * c(0x3f33_3333);
        let p4 = tri(raw.mul_add(c(0x402c_cccd), c(0xbe4c_cccd))) * c(0x3f4c_cccd);
        let sp = self.g[G_SPREAD].cur as f64;
        let s2 = sp.mul_add(sp, 0.0) as f32 as f64;
        let third = d(0x3fd5_5555_5555_5555);
        let pan = |u: f64| (u * third) * (4.0 - u);
        let l1 = band[0][0] * pan(1.0 - s2);
        let r1 = band[0][1] * pan(s2 + 1.0);
        let shapes = [p2, p3, p4].map(f64::from);
        let mut lk = [0.0f64; 3];
        let mut rk = [0.0f64; 3];
        for k in 0..3 {
            lk[k] = pan(1.0 - shapes[k]) * band[k + 1][0] * sp;
            rk[k] = pan(shapes[k] + 1.0) * band[k + 1][1] * sp;
        }
        // 6. The engage envelope.
        let e = self.g[G_ENGAGE].cur.abs() as f64;
        let k = if self.env < e { self.attack } else { self.release };
        self.env = (e - self.env).mul_add(k, self.env);
        let level = sp.mul_add(d(0xbfe6_6666_8000_0000), d(0x3ffb_3333_4000_0000)) as f32;
        let sum_l = lk[2] + (lk[1] + (lk[0] + l1));
        let sum_r = rk[2] + (rk[1] + (rk[0] + r1));
        let ring_l = (sum_l * d(0x3fe2_e147_a000_0000)) as f32;
        let ring_r = (sum_r * d(0x3fe2_e147_a000_0000)) as f32;
        let env = self.env as f32 as f64;
        let wet = level * (env.mul_add(self.g[G_WET].cur as f64, 0.0) as f32);
        let (ring_l, ring_r) = (wet * ring_l, wet * ring_r);
        // 7. The reverb, fully wet, and the mix.
        let (mut rl, mut rr) = ([ring_l], [ring_r]);
        self.plate.process(&mut rl, &mut rr);
        let (mix, dry) = (self.g[G_MIX].cur, self.g[G_DRY].cur);
        let dry = env.mul_add((dry - mix) as f64, mix as f64) as f32;
        let m = self.g[G_REV_MIX].cur;
        let rev = clamp01(m + m);
        let n = 1.0 - m;
        let direct = clamp01(n + n);
        (dry.mul_add(l, rl[0] * rev) + ring_l * direct, dry.mul_add(r, rr[0] * rev) + ring_r * direct)
    }
}

impl Mode for Ring {
    fn set(&mut self, p: &Params) {
        let v = [
            p.f("touchstrip_pos"),
            if p.f("touchstrip_on_off") >= 0.5 { 1.0 } else { 0.0 },
            p.f("ring_spread"),
            p.i("ring_mix_mode").min(1) as f32,
            p.f("ring_dry_wet"),
            p.f("ring_rev_mix"),
            p.f("ring_rev_time"),
            p.f("ring_glide"),
        ];
        if !self.seen {
            // Loaded: every parameter lands at once; then onReset and onResume.
            self.seen = true;
            for (i, x) in v.into_iter().enumerate() {
                self.set_parameter(i, x);
            }
            self.set_sample_rate(self.sr);
            self.reset();
            self.immediate = false;
            self.plate.resume();
            return;
        }
        for (i, x) in v.into_iter().enumerate() {
            if x != self.p[i] {
                self.set_parameter(i, x);
            }
        }
    }

    fn frame(&mut self, l: f32, r: f32, _c: &Ctx) -> (f32, f32) {
        if self.until == 0 {
            self.plate.control();
            self.until = BLOCK;
        }
        self.until -= 1;
        self.sample(l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(kv: &[(&str, f64)]) -> Vec<(String, f64)> {
        let mut p: Vec<(String, f64)> = deckard_model::PERFORM_FX.1.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        for (k, v) in kv {
            match p.iter_mut().find(|(n, _)| n == k) {
                Some(x) => x.1 = *v,
                None => p.push((k.to_string(), *v)),
            }
        }
        p
    }

    fn ctx(p: &[(String, f64)], sr: f32) -> Ctx {
        let v = |k: &str| p.iter().find(|(n, _)| n == k).map_or(0.0, |x| x.1) as f32;
        Ctx { sr, pos: v("touchstrip_pos"), engaged: v("touchstrip_on_off") >= 0.5, quarter: sr / 2.0, song: None }
    }

    /// A WAV's frames (as maschine-ref writes them), or None if it isn't there.
    fn wav(path: &str) -> Option<(Vec<[f32; 2]>, f32)> {
        let mut r = hound::WavReader::open(path).ok()?;
        let spec = r.spec();
        let ch = spec.channels as usize;
        let v: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => r.samples::<f32>().map(Result::unwrap).collect(),
            hound::SampleFormat::Int => {
                r.samples::<i32>().map(|s| s.unwrap() as f32 / (1u64 << (spec.bits_per_sample - 1)) as f32).collect()
            }
        };
        Some((v.chunks_exact(ch).map(|f| [f[0], f[ch - 1]]).collect(), spec.sample_rate as f32))
    }

    /// The worst sample of the difference (dB) against Maschine's render `name`, after
    /// the same second of silence; None if the render isn't there.
    fn null(name: &str, kv: &[(&str, f64)]) -> Option<f64> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../out/perform-ring");
        let (input, sr) = wav(&format!("{dir}/{name}-in.wav"))?;
        let (want, _) = wav(&format!("{dir}/{name}.wav"))?;
        let p = params(&[&[("mode", 4.0), ("touchstrip_on_off", 1.0)], kv].concat());
        let mut f = Ring::new(sr);
        f.set(&Params(&p));
        let c = ctx(&p, sr);
        for _ in 0..sr as usize {
            f.frame(0.0, 0.0, &c);
        }
        let mut worst = 0f32;
        for (x, w) in input.iter().zip(&want) {
            let (l, r) = f.frame(x[0], x[1], &c);
            worst = worst.max((l - w[0]).abs()).max((r - w[1]).abs());
        }
        Some(20.0 * (worst.max(1e-30) as f64).log10())
    }

    /// Maschine's renders (out/perform-ring/, docs/fx-perform-ring.md §6): the worst
    /// sample of the difference, per case. Skipped where the renders aren't there.
    #[test]
    fn nulls_against_maschine() {
        let cases: &[(&str, &[(&str, f64)])] = &[
            ("dry", &[("touchstrip_pos", 0.2), ("ring_spread", 0.0), ("ring_rev_mix", 0.0), ("ring_dry_wet", 1.0)]),
            ("spread", &[("touchstrip_pos", 0.8), ("ring_spread", 1.0), ("ring_rev_mix", 0.0), ("ring_dry_wet", 0.7)]),
            ("def", &[]),
            ("rev", &[("ring_rev_mix", 1.0), ("ring_rev_time", 0.8), ("ring_dry_wet", 0.6)]),
            ("wet", &[("ring_mix_mode", 1.0), ("ring_dry_wet", 0.5), ("ring_spread", 0.6)]),
            ("s48", &[("touchstrip_pos", 0.35), ("ring_glide", 0.5), ("ring_rev_mix", 0.3)]),
            ("pos0", &[("touchstrip_pos", 0.0), ("ring_spread", 0.05), ("ring_dry_wet", 0.9), ("ring_rev_mix", 0.2)]),
            ("pos1", &[("touchstrip_pos", 1.0), ("ring_spread", 0.15), ("ring_dry_wet", 0.25), ("ring_rev_time", 1.0)]),
            ("imp", &[("ring_rev_mix", 0.8), ("ring_rev_time", 0.5), ("ring_dry_wet", 1.0), ("touchstrip_pos", 0.6)]),
            ("dw0", &[("ring_dry_wet", 0.0), ("ring_rev_mix", 0.7), ("ring_rev_time", 0.0)]),
            ("r96", &[("touchstrip_pos", 0.45), ("ring_spread", 0.9), ("ring_glide", 1.0), ("ring_mix_mode", 1.0)]),
            ("r88", &[("touchstrip_pos", 0.7), ("ring_spread", 0.4), ("ring_glide", 0.005), ("ring_rev_mix", 1.0)]),
            ("off", &[("touchstrip_on_off", 0.0), ("ring_rev_mix", 0.6)]),
            ("offwet", &[("touchstrip_on_off", 0.0), ("ring_mix_mode", 1.0)]),
        ];
        let mut failed = Vec::new();
        for (name, kv) in cases {
            if let Some(db) = null(name, kv) {
                eprintln!("ring null {name}: worst {db:.1} dB");
                if db > -200.0 {
                    failed.push(format!("{name}: {db:.1} dB"));
                }
            }
        }
        assert!(failed.is_empty(), "{failed:?}");
    }

    /// Released after playing: the ring sound fades (100 ms), the reverb's tail dies away,
    /// and the input passes at unity in Mix mode.
    #[test]
    fn released_the_input_passes() {
        let sr = 44_100.0;
        let on = params(&[("mode", 4.0), ("touchstrip_on_off", 1.0), ("ring_dry_wet", 1.0)]);
        let mut f = Ring::new(sr);
        f.set(&Params(&on));
        let c = ctx(&on, sr);
        let x = |i: usize| ((i as f32) * 0.05).sin() * 0.5;
        for i in 0..20_000 {
            f.frame(x(i), x(i), &c);
        }
        f.set(&Params(&params(&[("mode", 4.0), ("touchstrip_on_off", 0.0), ("ring_dry_wet", 1.0)])));
        let mut late = 0f32;
        for i in 20_000..20_000 + 10 * 44_100 {
            let (l, r) = f.frame(x(i), x(i), &c);
            if i > 20_000 + 9 * 44_100 {
                late = late.max((l - x(i)).abs()).max((r - x(i)).abs());
            }
        }
        assert!(late < 1e-4, "{late}");
    }

    /// Engaged, the ring modulator moves energy away from a pure tone's frequency; Wet
    /// only, released, it is silent (as Maschine).
    #[test]
    fn engaged_it_rings_and_wet_only_released_is_silent() {
        let sr = 44_100.0;
        let p = params(&[("mode", 4.0), ("touchstrip_on_off", 1.0), ("ring_dry_wet", 1.0), ("ring_rev_mix", 0.0)]);
        let mut f = Ring::new(sr);
        f.set(&Params(&p));
        let c = ctx(&p, sr);
        let mut e = 0.0f64;
        for i in 0..44_100 {
            let x = ((i as f32) * 0.06).sin() * 0.5;
            let (l, _) = f.frame(x, x, &c);
            if i > 4_410 {
                e += (l as f64).powi(2);
            }
        }
        assert!(e > 1e-3, "{e}");
        let p = params(&[("mode", 4.0), ("touchstrip_on_off", 0.0), ("ring_mix_mode", 1.0)]);
        let mut f = Ring::new(sr);
        f.set(&Params(&p));
        let worst = (0..10_000).map(|_| f.frame(0.3, -0.3, &c)).fold(0f32, |m, (l, r)| m.max(l.abs()).max(r.abs()));
        assert_eq!(worst, 0.0);
    }
}
